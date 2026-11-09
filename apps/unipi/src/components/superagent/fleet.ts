import type { Fleet, FleetNode, SuperagentSession, SuperagentStatus } from './api';

/**
 * The agent canvas, minus the DOM: the fleet the host reports (or, on an older
 * host, what the status + history calls already say), grouped per project,
 * filtered, laid out as one dashed region per project, and the view maths.
 * Everything here is pure so it is tested without a renderer.
 */

export type FleetStatus = FleetNode['status'];
export type FleetFilter = FleetStatus | 'all';
export const FILTERS: readonly FleetFilter[] = ['working', 'waiting', 'idle', 'ended', 'all'];
export const DEFAULT_FILTER: FleetFilter = 'working';
export const STATUS_LABEL: Record<FleetStatus, string> = { working: 'Working', waiting: 'Waiting for you', idle: 'Idle', ended: 'Ended' };
export const FILTER_LABEL: Record<FleetFilter, string> = { working: 'Working', waiting: 'Waiting', idle: 'Idle', ended: 'Ended', all: 'All' };
export const MODE_BADGE: Record<FleetNode['mode'], string> = { plain: 'Plain', graph: 'Graph', swarm: 'Swarm' };

export const UNGROUPED = 'Ungrouped';
/** The fallback lists at most this many finished sessions: a host with a long history would otherwise bury the canvas. */
export const FALLBACK_ENDED_MAX = 40;

// ---- Fallback for hosts without `superagent.fleet` ---------------------------------------------

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
    const project = projectName(lead.projectId);
    nodes.set(lead.sessionId, {
      id: lead.sessionId, parentId: null, project, projectId: lead.projectId, title: `Lead · ${project}`,
      mode: lead.mode === 'graph' || lead.mode === 'swarm' ? lead.mode : 'plain', role: 'lead',
      status: waiting.has(lead.sessionId) ? 'waiting' : 'working', model: lead.model ?? null, startedAt: time(lead.startedAt), endedAt: null,
    });
  }
  const rows = [...(history ?? [])];
  const ended: FleetNode[] = [];
  for (const item of rows) {
    if (nodes.has(item.sessionId)) continue;
    const running = item.state?.kind === 'working' || !!item.pendingWork;
    const node: FleetNode = {
      id: item.sessionId, parentId: null, project: item.project ?? null, projectId: projectIdOf(item.project), title: item.title || 'Superagent',
      mode: 'plain', role: 'lead', status: running ? (waiting.has(item.sessionId) ? 'waiting' : 'working') : 'ended', model: null,
      startedAt: time(item.createdAt), endedAt: running ? null : time(item.updatedAt),
    };
    if (running) nodes.set(node.id, node); else ended.push(node);
  }
  ended.sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));
  for (const node of ended.slice(0, FALLBACK_ENDED_MAX)) nodes.set(node.id, node);
  return { nodes: [...nodes.values()], edges: [] };
}

// ---- Counts and filters ---------------------------------------------------------------------------

export function dedupeNodes(nodes: readonly FleetNode[]): FleetNode[] {
  const byId = new Map<string, FleetNode>();
  for (const node of nodes) byId.set(node.id, node); // the later report wins, the first position stays
  return [...byId.values()];
}

export function fleetCounts(nodes: readonly FleetNode[]): Record<FleetFilter, number> {
  const out: Record<FleetFilter, number> = { working: 0, waiting: 0, idle: 0, ended: 0, all: 0 };
  for (const node of dedupeNodes(nodes)) { out[node.status]++; out.all++; }
  return out;
}

// ---- Tree: who sits under whom, and in which region -------------------------------------------------

export type FleetGroup = { key: string; name: string; ungrouped: boolean; roots: string[] };
export type FleetTree = {
  nodes: Map<string, FleetNode>;
  /** Effective parent after orphan re-attachment; null for a root. */
  parent: Map<string, string | null>;
  children: Map<string, string[]>;
  /** Region key of every node. */
  group: Map<string, string>;
  /** Regions in first-seen order, Ungrouped last. */
  groups: FleetGroup[];
};

const projectKey = (n: FleetNode): string | null => n.projectId || n.project || null;

export function buildTree(fleet: Pick<Fleet, 'nodes'> & Partial<Pick<Fleet, 'edges'>>): FleetTree {
  const list = dedupeNodes(fleet.nodes);
  const nodes = new Map(list.map((n) => [n.id, n]));
  const leadOf = new Map<string, string>();
  for (const n of list) { const k = projectKey(n); if (k && n.role === 'lead' && !leadOf.has(k)) leadOf.set(k, n.id); }
  const edgeParent = new Map<string, string>();
  for (const e of fleet.edges ?? []) if (nodes.has(e.from) && nodes.has(e.to) && e.from !== e.to && !edgeParent.has(e.to)) edgeParent.set(e.to, e.from);

  const parent = new Map<string, string | null>();
  /** Nodes that lost their parent and have no project lead to hang from. */
  const stray = new Set<string>();
  for (const n of list) {
    const declared = n.parentId || edgeParent.get(n.id) || null;
    if (declared && declared !== n.id && nodes.has(declared)) { parent.set(n.id, declared); continue; }
    if (declared && n.role !== 'lead') {
      const lead = leadOf.get(projectKey(n) ?? '');
      if (lead && lead !== n.id) { parent.set(n.id, lead); continue; }
      stray.add(n.id);
    }
    parent.set(n.id, null);
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
  const group = new Map<string, string>();
  const names = new Map<string, string>();
  const order: string[] = [];
  for (const n of list) {
    const root = nodes.get(rootOf(n.id))!;
    const key = stray.has(root.id) ? null : projectKey(root);
    const g = key ?? UNGROUPED;
    group.set(n.id, g);
    if (!names.has(g)) { names.set(g, key ? (root.project || key) : UNGROUPED); order.push(g); }
  }
  const children = new Map<string, string[]>(list.map((n) => [n.id, []]));
  for (const n of list) { const p = parent.get(n.id); if (p) children.get(p)!.push(n.id); }

  const keys = [...order.filter((k) => k !== UNGROUPED), ...order.filter((k) => k === UNGROUPED)];
  const groups: FleetGroup[] = keys.map((key) => {
    const roots = list.filter((n) => !parent.get(n.id) && group.get(n.id) === key);
    roots.sort((a, b) => Number(b.role === 'lead') - Number(a.role === 'lead')); // stable: leads first
    return { key, name: names.get(key)!, ungrouped: key === UNGROUPED, roots: roots.map((n) => n.id) };
  });
  return { nodes, parent, children, group, groups };
}

export type Filtered = { visible: Set<string>; dimmed: Set<string>; matches: number };

/** Matching nodes plus their ancestors (dimmed), so a kept node is never cut loose from its tree. */
export function filterTree(tree: FleetTree, filter: FleetFilter): Filtered {
  const visible = new Set<string>();
  const dimmed = new Set<string>();
  let matches = 0;
  for (const [id, node] of tree.nodes) {
    if (filter !== 'all' && node.status !== filter) continue;
    matches++;
    visible.add(id);
    for (let p = tree.parent.get(id); p && !visible.has(p); p = tree.parent.get(p)) { visible.add(p); dimmed.add(p); }
  }
  // An ancestor that matches on its own is not dimmed, whichever was visited first.
  for (const id of [...dimmed]) { const n = tree.nodes.get(id)!; if (filter === 'all' || n.status === filter) dimmed.delete(id); }
  return { visible, dimmed, matches };
}

// ---- Layout ---------------------------------------------------------------------------------------------

export const CARD_W = 200;
export const CARD_H = 64;
export const GAP_X = 20;
export const GAP_Y = 44;
export const REGION_PAD = 18;
export const REGION_TOP = 44;
export const REGION_GAP = 28;
export const WRAP_WIDTH = 1480;

export type Rect = { x: number; y: number; w: number; h: number };
export type PlacedRegion = Rect & { key: string; name: string; ungrouped: boolean };
export type PlacedNode = Rect & { id: string; region: string; depth: number };
export type PlacedEdge = { from: string; to: string; d: string; dimmed: boolean };
export type Layout = { width: number; height: number; regions: PlacedRegion[]; nodes: PlacedNode[]; edges: PlacedEdge[] };

const labelWidth = (name: string) => Math.min(360, name.length * 8 + 2 * REGION_PAD + 12);

export function layoutFleet(tree: FleetTree, filtered?: Pick<Filtered, 'visible' | 'dimmed'>, wrapWidth = WRAP_WIDTH): Layout {
  const shown = (id: string) => !filtered || filtered.visible.has(id);
  const kids = (id: string) => (tree.children.get(id) ?? []).filter(shown);
  const widths = new Map<string, number>();
  const subtree = (id: string): number => {
    const cached = widths.get(id);
    if (cached !== undefined) return cached;
    const c = kids(id);
    const w = c.length ? Math.max(CARD_W, c.reduce((sum, k) => sum + subtree(k), 0) + GAP_X * (c.length - 1)) : CARD_W;
    widths.set(id, w);
    return w;
  };

  type Local = { id: string; x: number; y: number; depth: number };
  const regions: PlacedRegion[] = [];
  const nodes: PlacedNode[] = [];
  const edges: PlacedEdge[] = [];
  const parts: { group: FleetGroup; w: number; h: number; local: Local[] }[] = [];

  for (const group of tree.groups) {
    const roots = group.roots.filter(shown);
    if (!roots.length) continue;
    const local: Local[] = [];
    let maxDepth = 0;
    const place = (id: string, left: number, depth: number): number => {
      const w = subtree(id);
      maxDepth = Math.max(maxDepth, depth);
      const slot: Local = { id, x: 0, y: depth * (CARD_H + GAP_Y), depth };
      local.push(slot); // parent first: depth-first reading order for Tab
      const c = kids(id);
      if (!c.length) { slot.x = left + (w - CARD_W) / 2; return slot.x + CARD_W / 2; }
      let x = left;
      const centres = c.map((k) => { const centre = place(k, x, depth + 1); x += subtree(k) + GAP_X; return centre; });
      slot.x = (centres[0]! + centres[centres.length - 1]!) / 2 - CARD_W / 2;
      return slot.x + CARD_W / 2;
    };
    let x = 0;
    for (const root of roots) { place(root, x, 0); x += subtree(root) + GAP_X; }
    const forestW = x - GAP_X;
    const w = Math.max(forestW + 2 * REGION_PAD, labelWidth(group.name));
    const dx = REGION_PAD + (w - 2 * REGION_PAD - forestW) / 2;
    for (const slot of local) { slot.x += dx; slot.y += REGION_TOP; }
    parts.push({ group, w, h: REGION_TOP + (maxDepth + 1) * CARD_H + maxDepth * GAP_Y + REGION_PAD, local });
  }

  let x = 0; let y = 0; let rowH = 0; let width = 0;
  for (const part of parts) {
    if (x > 0 && x + part.w > wrapWidth) { x = 0; y += rowH + REGION_GAP; rowH = 0; }
    regions.push({ key: part.group.key, name: part.group.name, ungrouped: part.group.ungrouped, x, y, w: part.w, h: part.h });
    for (const slot of part.local) nodes.push({ id: slot.id, region: part.group.key, depth: slot.depth, x: x + slot.x, y: y + slot.y, w: CARD_W, h: CARD_H });
    x += part.w + REGION_GAP; rowH = Math.max(rowH, part.h); width = Math.max(width, x - REGION_GAP);
  }
  const height = parts.length ? y + rowH : 0;

  const at = new Map(nodes.map((n) => [n.id, n]));
  for (const n of nodes) {
    const p = tree.parent.get(n.id);
    const from = p ? at.get(p) : undefined;
    if (!from) continue;
    const x1 = from.x + CARD_W / 2; const y1 = from.y + CARD_H; const x2 = n.x + CARD_W / 2; const y2 = n.y;
    const mid = y1 + GAP_Y / 2;
    edges.push({ from: from.id, to: n.id, d: x1 === x2 ? `M${x1} ${y1}V${y2}` : `M${x1} ${y1}V${mid}H${x2}V${y2}`, dimmed: !!filtered?.dimmed.has(n.id) });
  }
  return { width, height, regions, nodes, edges };
}

// ---- View maths ------------------------------------------------------------------------------------------

export type View = { x: number; y: number; scale: number };
export const MIN_SCALE = 0.25;
export const MAX_SCALE = 2;
export const FIT_PADDING = 40;
/** Scale 1 with the world's origin a little inside the viewport's corner. */
export const RESET_VIEW: View = { x: 24, y: 24, scale: 1 };
export const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Everything visible, centred, never magnified past 1 (one node should not fill the screen). */
export function fitView(content: { width: number; height: number }, viewport: { width: number; height: number }, padding = FIT_PADDING): View {
  if (content.width <= 0 || content.height <= 0 || viewport.width <= 0 || viewport.height <= 0) return RESET_VIEW;
  const scale = Math.min(1, clampScale(Math.min((viewport.width - 2 * padding) / content.width, (viewport.height - 2 * padding) / content.height)));
  return { scale, x: (viewport.width - content.width * scale) / 2, y: (viewport.height - content.height * scale) / 2 };
}

/** Zoom keeping the world point under (cx, cy) fixed. */
export function zoomAt(view: View, factor: number, cx: number, cy: number): View {
  const scale = clampScale(view.scale * factor);
  const k = scale / view.scale;
  return { scale, x: cx - (cx - view.x) * k, y: cy - (cy - view.y) * k };
}

// ---- Opening a node -----------------------------------------------------------------------------------------

/**
 * The session a click opens. Leads and workers open themselves; a subagent has no chat of its own
 * in the Superagent pane, so it opens the nearest lead/worker above it (itself when it is a stray root).
 */
export function openTarget(tree: FleetTree, id: string): string {
  for (let cur: string | null | undefined = id; cur; cur = tree.parent.get(cur)) {
    if (tree.nodes.get(cur)?.role !== 'subagent') return cur;
  }
  return id;
}
