/**
 * Long-horizon graph / swarm data for the app ([lh-ui]): the `long-horizon.graph` service's
 * `snapshot` / `history` replies (docs/dev/v2/qa/long-horizon-errored.md), parsed defensively,
 * plus the pure view-shaping every LH view shares: status meta, the "Show errors" filter,
 * newest-first order, the progress segments, per-node elapsed time and the merged event
 * timeline. No UI, no I/O here.
 */

/** Display order (also the `counts` keys). */
export const LH_STATUSES = ['queued', 'running', 'blocked', 'errored', 'completed', 'failed', 'stopped', 'cancelled', 'superseded'] as const;
export type LhStatus = (typeof LH_STATUSES)[number];
export type LhBy = 'lead' | 'user' | 'auto' | 'host';
export type LhTone = 'wait' | 'run' | 'warn' | 'error' | 'ok' | 'fail' | 'off';

export type LhStatusMeta = { label: string; tone: LhTone };
export const STATUS_META: Record<LhStatus, LhStatusMeta> = {
  queued: { label: 'Queued', tone: 'wait' },
  running: { label: 'Running', tone: 'run' },
  blocked: { label: 'Blocked', tone: 'wait' },
  errored: { label: 'Error', tone: 'error' },
  completed: { label: 'Done', tone: 'ok' },
  failed: { label: 'Failed', tone: 'fail' },
  stopped: { label: 'Stopped', tone: 'off' },
  cancelled: { label: 'Cancelled', tone: 'off' },
  superseded: { label: 'Replaced', tone: 'off' },
};

/** Statuses the "Show errors" toggle hides when off. */
export const ERROR_STATUSES: readonly LhStatus[] = ['errored', 'failed', 'superseded'];

export function asStatus(v: unknown): LhStatus {
  const s = typeof v === 'string' ? v : '';
  if (s === 'done') return 'completed';
  return (LH_STATUSES as readonly string[]).includes(s) ? (s as LhStatus) : 'queued';
}

export type LhWorkEvent = { ts: number; status: string; by: LhBy; detail?: string };
export type LhGraphEvent = { ts: number; kind: string; by: LhBy; detail?: string };

export type LhWork = {
  workId: string;
  status: LhStatus;
  instruction: string;
  profile?: string;
  model?: string;
  thinking?: string;
  inputIds: string[];
  attempts: number;
  autoContinues: number;
  continues: number;
  toolCalls: number;
  canRetry: boolean;
  hasOutput: boolean;
  events: LhWorkEvent[];
  eventsDropped: number;
  operatorId?: string;
  resultId?: string;
  failureClass?: string;
  error?: string;
  stopReason?: string;
  replaces?: string;
  supersededBy?: string;
  blockedBy?: string;
  nextRetryAt?: number;
  /** USD, when the host reports it (not in the v1 schema; shown only when present). */
  cost?: number;
  /**
   * The graph moved on from this step (stopped / replaced / settled) but its worker still
   * runs: the host reports it as `running` with `orphaned`, `stoppedAs` (the graph's own
   * status) and `canStop` (the `stop_operator` op).
   */
  orphaned?: boolean;
  stoppedAs?: string;
  canStop?: boolean;
};

export type LhGraphCounts = Record<LhStatus, number>;

export type LhGraphSnap = {
  graphId: string;
  mode: 'graph' | 'swarm';
  task: string;
  state: 'open' | 'finished' | 'stopped';
  revision: number;
  createdAt: number;
  updatedAt: number;
  workCount: number;
  counts: LhGraphCounts;
  events: LhGraphEvent[];
  eventsDropped: number;
  work: LhWork[];
  finishReason?: string;
  finishAt?: number;
  abandoned?: string;
  omittedWork: number;
  hasCounts: boolean;
};

export type LhHistoryRow = Omit<LhGraphSnap, 'events' | 'eventsDropped' | 'work' | 'omittedWork' | 'hasCounts'>;

const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);
const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
const by = (v: unknown): LhBy => (v === 'user' || v === 'auto' || v === 'host' ? v : 'lead');

export function emptyCounts(): LhGraphCounts {
  return Object.fromEntries(LH_STATUSES.map((s) => [s, 0])) as LhGraphCounts;
}

function parseCounts(v: unknown): LhGraphCounts {
  const out = emptyCounts();
  const o = obj(v) ?? {};
  for (const s of LH_STATUSES) out[s] = Math.max(0, Math.floor(num(o[s]) ?? 0));
  return out;
}

function parseWork(raw: unknown): LhWork | undefined {
  const o = obj(raw);
  const workId = str(o?.workId);
  if (!o || !workId) return undefined;
  const events: LhWorkEvent[] = [];
  for (const e of Array.isArray(o.events) ? o.events.slice(-24) : []) {
    const eo = obj(e);
    const ts = num(eo?.ts);
    const status = str(eo?.status);
    if (eo && ts !== undefined && status) events.push({ ts, status, by: by(eo.by), ...(str(eo.detail) ? { detail: str(eo.detail) } : {}) });
  }
  const status = asStatus(o.status);
  const w: LhWork = {
    workId,
    status,
    instruction: str(o.instruction) ?? '',
    inputIds: Array.isArray(o.inputIds) ? o.inputIds.filter((i): i is string => typeof i === 'string') : [],
    attempts: num(o.attempts) ?? 0,
    autoContinues: num(o.autoContinues) ?? 0,
    continues: num(o.continues) ?? 0,
    toolCalls: num(o.toolCalls) ?? 0,
    canRetry: o.canRetry === true || (o.canRetry === undefined && status === 'errored'),
    hasOutput: o.hasOutput === true,
    events,
    eventsDropped: num(o.eventsDropped) ?? 0,
  };
  for (const k of ['profile', 'model', 'thinking', 'operatorId', 'resultId', 'failureClass', 'error', 'stopReason', 'replaces', 'supersededBy', 'blockedBy'] as const) {
    const v = str(o[k]);
    if (v) w[k] = v;
  }
  const retryAt = num(o.nextRetryAt);
  if (retryAt) w.nextRetryAt = retryAt;
  const cost = num(o.cost);
  if (cost !== undefined && cost >= 0) w.cost = cost;
  if (o.orphaned === true) {
    w.orphaned = true;
    w.canStop = o.canStop !== false;
    const was = str(o.stoppedAs);
    if (was) w.stoppedAs = was;
  }
  return w;
}

function parseHead(o: Record<string, unknown>): LhHistoryRow | undefined {
  const graphId = str(o.graphId);
  if (!graphId) return undefined;
  const finish = obj(o.finish);
  const st = o.state === 'finished' || o.state === 'stopped' ? o.state : 'open';
  return {
    graphId,
    mode: o.mode === 'swarm' ? 'swarm' : 'graph',
    task: str(o.task) ?? graphId,
    state: st,
    revision: num(o.revision) ?? 0,
    createdAt: num(o.createdAt) ?? 0,
    updatedAt: num(o.updatedAt) ?? num(o.createdAt) ?? 0,
    workCount: num(o.workCount) ?? 0,
    counts: parseCounts(o.counts),
    ...(str(finish?.reason) ? { finishReason: str(finish?.reason) } : {}),
    ...(num(finish?.at) !== undefined ? { finishAt: num(finish?.at) } : {}),
    ...(str(o.abandoned) ? { abandoned: str(o.abandoned) } : {}),
  };
}

/** A `snapshot` reply → the graph, or undefined (`graph: null`, `ok:false`, malformed). Pure. */
export function parseSnapshot(reply: unknown): LhGraphSnap | undefined {
  const r = obj(reply);
  const g = obj(r?.graph);
  if (!r || r.ok === false || !g) return undefined;
  const head = parseHead(g);
  if (!head) return undefined;
  const events: LhGraphEvent[] = [];
  for (const e of Array.isArray(g.events) ? g.events.slice(-48) : []) {
    const eo = obj(e);
    const ts = num(eo?.ts);
    const kind = str(eo?.kind);
    if (eo && ts !== undefined && kind) events.push({ ts, kind, by: by(eo.by), ...(str(eo.detail) ? { detail: str(eo.detail) } : {}) });
  }
  const work = (Array.isArray(g.work) ? g.work.slice(0, 120) : []).map(parseWork).filter((w): w is LhWork => !!w);
  return { ...head, events, eventsDropped: num(g.eventsDropped) ?? 0, work, omittedWork: num(g.omittedWork) ?? 0, hasCounts: !!obj(g.counts) };
}

/** A `history` reply → rows, newest first as sent (≤ 21). Pure. */
export function parseHistory(reply: unknown): LhHistoryRow[] {
  const r = obj(reply);
  if (!r || r.ok === false || !Array.isArray(r.graphs)) return [];
  return r.graphs.slice(0, 21).map((g) => (obj(g) ? parseHead(obj(g)!) : undefined)).filter((g): g is LhHistoryRow => !!g);
}

// ── Central error codes (`lh.<entity>.<reason>`), handled exhaustively ────────

export type LhCode =
  | 'lh.graph.not_found'
  | 'lh.graph.closed'
  | 'lh.work.not_found'
  | 'lh.work.not_continuable'
  | 'lh.work.not_failable'
  | 'lh.work.continue_limit'
  | 'lh.request.invalid'
  | 'lh.goal.not_found'
  | 'lh.goal.not_errored';

export const LH_CODES: readonly LhCode[] = [
  'lh.graph.not_found',
  'lh.graph.closed',
  'lh.work.not_found',
  'lh.work.not_continuable',
  'lh.work.not_failable',
  'lh.work.continue_limit',
  'lh.request.invalid',
  'lh.goal.not_found',
  'lh.goal.not_errored',
];

/** The user-facing text of a refusal code. A new code left unhandled fails the compile. */
export function lhCodeText(code: LhCode, reason?: string): string {
  switch (code) {
    case 'lh.graph.not_found':
      return 'That graph is gone.';
    case 'lh.graph.closed':
      return 'This graph is closed.';
    case 'lh.work.not_found':
      return 'That step is gone.';
    case 'lh.work.not_continuable':
      return reason ?? 'Already retrying.';
    case 'lh.work.not_failable':
      return reason ?? 'That step is already settled.';
    case 'lh.work.continue_limit':
      return 'Retried too often — ask the agent to replace it.';
    case 'lh.request.invalid':
      return `Invalid request${reason ? `: ${reason}` : ''} (${code})`;
    case 'lh.goal.not_found':
      return 'No goal is running.';
    case 'lh.goal.not_errored':
      return 'Nothing to retry.';
    default: {
      const never: never = code;
      return String(never);
    }
  }
}

/** The text for a refusal reply `{ok:false, code, reason}`; unknown codes show the reason. Pure. */
export function refusalText(reply: unknown): string | undefined {
  const r = obj(reply);
  if (!r || r.ok !== false) return undefined;
  const code = str(r.code);
  const reason = str(r.reason);
  if (code && (LH_CODES as readonly string[]).includes(code)) return lhCodeText(code as LhCode, reason);
  return reason ?? 'The request did not go through.';
}

// ── View shaping ──────────────────────────────────────────────────────────

/** When a work item last changed (its newest event; else 0). */
export function lastTouch(w: LhWork): number {
  return w.events.length ? w.events[w.events.length - 1]!.ts : 0;
}

/** Newest first (owner 2026-11-04: the running one was at the bottom). Items without events keep
 *  the reverse of their creation order. Stable and pure. */
export function newestFirst(work: readonly LhWork[]): LhWork[] {
  return work
    .map((w, i) => ({ w, i, t: lastTouch(w) }))
    .sort((a, b) => b.t - a.t || b.i - a.i)
    .map((x) => x.w);
}

/** The "Show errors" filter: off hides errored / failed / superseded. Pure. */
export function visibleWork(work: readonly LhWork[], showErrors: boolean): LhWork[] {
  return showErrors ? [...work] : work.filter((w) => !ERROR_STATUSES.includes(w.status));
}

export type NodeCategory = 'stopped' | 'replaced' | 'blocked' | 'done' | 'active' | 'errors';
export const FILTERS: readonly NodeCategory[] = ['stopped', 'replaced', 'blocked', 'done', 'active', 'errors'];
export function nodeCategory(node: Pick<LhWork, 'status'>): NodeCategory {
  switch (node.status) {
    case 'stopped': case 'cancelled': return 'stopped';
    case 'superseded': return 'replaced';
    case 'blocked': return 'blocked';
    case 'completed': return 'done';
    case 'errored': case 'failed': return 'errors';
    default: return 'active';
  }
}
export function matchesNodeFilter(node: Pick<LhWork, 'status'>, filters: readonly NodeCategory[]): boolean {
  return filters.length === 0 || filters.includes(nodeCategory(node));
}
export function graphProgress(graph: Pick<LhGraphSnap, 'work' | 'counts' | 'hasCounts'>) {
  const c = graph.hasCounts ? graph.counts : countWork(graph.work);
  const done = c.completed, stopped = c.stopped + c.cancelled, replaced = c.superseded;
  const blocked = c.blocked, errored = c.errored, failed = c.failed, active = c.running + c.queued;
  const total = done + stopped + replaced + blocked + errored + failed + active;
  return { done, stopped, replaced, blocked, errored, failed, active, total, denom: total - stopped - replaced, finished: active === 0 };
}

export type LhSegment = { status: LhStatus; count: number; pct: number };

/** Counts of a work list per status (zero keys kept). */
export function countWork(work: readonly LhWork[]): LhGraphCounts {
  const c = emptyCounts();
  for (const w of work) c[w.status] += 1;
  return c;
}

/** The bar's coloured segments (done · running · error · failed; the rest is the track). */
export function progressSegments(counts: LhGraphCounts): LhSegment[] {
  const total = LH_STATUSES.reduce((n, s) => n + counts[s], 0);
  if (total <= 0) return [];
  const order: LhStatus[] = ['completed', 'running', 'errored', 'failed'];
  return order.filter((s) => counts[s] > 0).map((s) => ({ status: s, count: counts[s], pct: (counts[s] / total) * 100 }));
}

/** Total of a counts record. */
export const countTotal = (c: LhGraphCounts): number => LH_STATUSES.reduce((n, s) => n + c[s], 0);

/** Time a node spent running: the sum of running → next-event spans (live to `now` when the last
 *  event is `running`); undefined when it never ran. Pure. */
export function workElapsedMs(w: Pick<LhWork, 'events'>, now: number): number | undefined {
  let total = 0;
  let ran = false;
  for (let i = 0; i < w.events.length; i++) {
    const e = w.events[i]!;
    if (e.status !== 'running') continue;
    ran = true;
    const next = w.events[i + 1];
    total += Math.max(0, (next ? next.ts : now) - e.ts);
  }
  return ran ? total : undefined;
}

export type LhTimelineEntry = {
  id: string;
  ts: number;
  /** Event kind / status: opened, running, errored, continued, completed, failed, checkpoint, finished … */
  kind: string;
  by: LhBy;
  workId?: string;
  detail?: string;
  tone: LhTone;
  title: string;
};

function kindTone(kind: string): LhTone {
  switch (kind) {
    case 'running':
      return 'run';
    case 'errored':
      return 'error';
    case 'completed':
    case 'finished':
      return 'ok';
    case 'failed':
      return 'fail';
    case 'stopped':
    case 'superseded':
      return 'off';
    case 'continued':
      return 'warn';
    case 'checkpoint':
      return 'run';
    default:
      return 'wait';
  }
}

const KIND_TITLE: Record<string, string> = {
  opened: 'Opened',
  queued: 'Queued',
  running: 'Running',
  errored: 'Error',
  continued: 'Continued',
  completed: 'Completed',
  failed: 'Failed',
  stopped: 'Stopped',
  superseded: 'Replaced',
  checkpoint: 'Checkpoint',
  finished: 'Finished',
};

/** "continued by user/lead/auto" and friends: the sentence the timeline shows. Pure. */
export function eventTitle(kind: string, actor: LhBy): string {
  const base = KIND_TITLE[kind] ?? kind;
  return kind === 'continued' ? `Continued by ${actor === 'host' ? 'the host' : actor}` : base;
}

/** The graph's and every node's events merged into one list, newest first. Pure. */
export function graphTimeline(g: Pick<LhGraphSnap, 'events' | 'work'>, showErrors = true): LhTimelineEntry[] {
  const out: LhTimelineEntry[] = [];
  g.events.forEach((e, i) =>
    out.push({ id: `g${i}`, ts: e.ts, kind: e.kind, by: e.by, tone: kindTone(e.kind), title: eventTitle(e.kind, e.by), ...(e.detail ? { detail: e.detail } : {}) }),
  );
  for (const w of g.work) {
    if (!showErrors && ERROR_STATUSES.includes(w.status)) continue;
    w.events.forEach((e, i) =>
      out.push({ id: `${w.workId}:${i}`, ts: e.ts, kind: e.status, by: e.by, workId: w.workId, tone: kindTone(e.status), title: eventTitle(e.status, e.by), ...(e.detail ? { detail: e.detail } : {}) }),
    );
  }
  return out.map((e, i) => ({ e, i })).sort((a, b) => b.e.ts - a.e.ts || b.i - a.i).map((x) => x.e);
}

/** `scan` retry text for logs: the first line, clipped. */
export const clip = (text: string, max: number): string => {
  const line = text.split('\n')[0] ?? '';
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
};

/** `subagent_explore` → `explore`; custom agent ids stay. */
export const shortProfile = (profile: string): string => profile.replace(/^subagent_/, '');

// ── DAG layout (the diagram) ──────────────────────────────────────────────

export const NODE_W = 200;
export const NODE_H = 76;
const GAP_X = 20;
const GAP_Y = 44;
const MAX_PER_ROW = 4;

export type LaidNode = { id: string; x: number; y: number; layer: number };
export type Pt = { x: number; y: number };
/** `points`: the routed polyline from the source's bottom side to the target's top side. */
export type LaidEdge = { from: string; to: string; kind: 'input' | 'replaces'; points: Pt[] };
export type Layout = { nodes: LaidNode[]; edges: LaidEdge[]; width: number; height: number };

/**
 * Layered top-down layout: a node's layer is its longest input chain; a layer wider than
 * `MAX_PER_ROW` wraps into rows (so 40 nodes stay a sensible aspect). A swarm (no edges)
 * is a plain grid of `MAX_PER_ROW`. Order inside a layer follows the given order. Pure.
 */
export function layoutGraph(work: readonly LhWork[], perRow = MAX_PER_ROW): Layout {
  const ids = new Set(work.map((w) => w.workId));
  const depth = new Map<string, number>();
  const byId = new Map(work.map((w) => [w.workId, w]));
  const visiting = new Set<string>();
  const layerOf = (id: string): number => {
    const known = depth.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return 0;
    visiting.add(id);
    let d = 0;
    for (const input of byId.get(id)?.inputIds ?? []) {
      const from = input.replace(/^r-/, '');
      if (ids.has(from)) d = Math.max(d, layerOf(from) + 1);
    }
    visiting.delete(id);
    depth.set(id, d);
    return d;
  };
  const layers = new Map<number, string[]>();
  for (const w of work) {
    const l = layerOf(w.workId);
    layers.set(l, [...(layers.get(l) ?? []), w.workId]);
  }
  const nodes: LaidNode[] = [];
  let y = 0;
  let width = 0;
  for (const l of [...layers.keys()].sort((a, b) => a - b)) {
    const list = layers.get(l)!;
    for (let r = 0; r * perRow < list.length; r++) {
      const row = list.slice(r * perRow, (r + 1) * perRow);
      const rowW = row.length * NODE_W + (row.length - 1) * GAP_X;
      width = Math.max(width, rowW);
      row.forEach((id, i) => nodes.push({ id, x: i * (NODE_W + GAP_X), y, layer: l }));
      y += NODE_H + GAP_Y;
    }
  }
  // Centre every row on the widest one.
  const rows = new Map<number, LaidNode[]>();
  for (const n of nodes) rows.set(n.y, [...(rows.get(n.y) ?? []), n]);
  for (const row of rows.values()) {
    const rowW = row.length * NODE_W + (row.length - 1) * GAP_X;
    const shift = (width - rowW) / 2;
    for (const n of row) n.x += shift;
  }
  const at = new Map(nodes.map((n) => [n.id, n]));
  const edges = graphEdges(work).map((e) => {
    const a = at.get(e.from)!;
    const b = at.get(e.to)!;
    const ax = a.x + NODE_W / 2;
    const bx = b.x + NODE_W / 2;
    const ay = a.y + NODE_H;
    const midY = ay + GAP_Y / 2;
    return { ...e, points: ax === bx ? [{ x: ax, y: ay }, { x: bx, y: b.y }] : [{ x: ax, y: ay }, { x: ax, y: midY }, { x: bx, y: midY }, { x: bx, y: b.y }] };
  });
  return { nodes, edges, width, height: Math.max(0, y - GAP_Y) };
}

/** The edges between the given work items (input → step, superseded → replacement). An edge to an
 *  item that is not in the list (hidden errors, trimmed history) is dropped, never left dangling. Pure. */
export function graphEdges(work: readonly LhWork[]): Omit<LaidEdge, 'points'>[] {
  const ids = new Set(work.map((w) => w.workId));
  const seen = new Set<string>();
  const out: Omit<LaidEdge, 'points'>[] = [];
  const add = (from: string, to: string, kind: LaidEdge['kind']) => {
    const k = `${kind}:${from}>${to}`;
    if (from === to || !ids.has(from) || !ids.has(to) || seen.has(k)) return;
    seen.add(k);
    out.push({ from, to, kind });
  };
  for (const w of work) {
    for (const input of w.inputIds) add(input.replace(/^r-/, ''), w.workId, 'input');
    if (w.supersededBy) add(w.workId, w.supersededBy, 'replaces');
  }
  return out;
}

/** A cache key of the graph's STRUCTURE (ids + edges): status changes do not re-layout. */
export const graphShapeKey = (work: readonly LhWork[]): string =>
  work.map((w) => w.workId).join('|') + '#' + graphEdges(work).map((e) => `${e.kind[0]}${e.from}>${e.to}`).join('|');

const ELK_OPTS = {
  'elk.algorithm': 'layered',
  'elk.direction': 'DOWN',
  'elk.edgeRouting': 'ORTHOGONAL',
  'elk.spacing.nodeNode': '28',
  'elk.spacing.edgeEdge': '14',
  'elk.spacing.edgeNode': '18',
  'elk.layered.spacing.nodeNodeBetweenLayers': '56',
  'elk.layered.spacing.edgeNodeBetweenLayers': '22',
  'elk.layered.spacing.edgeEdgeBetweenLayers': '14',
  'elk.layered.nodePlacement.strategy': 'NETWORK_SIMPLEX',
  'elk.layered.crossingMinimization.strategy': 'LAYER_SWEEP',
  'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
  'elk.aspectRatio': '1.1',
  'elk.padding': '[top=4,left=4,bottom=4,right=4]',
};

type ElkSection = { startPoint: Pt; endPoint: Pt; bendPoints?: Pt[] };
type ElkOut = { children?: { id: string; x?: number; y?: number }[]; edges?: { id: string; sections?: ElkSection[] }[]; width?: number; height?: number };

/**
 * The real layout: elkjs layered, orthogonal routing, one port per edge end (north = in,
 * south = out) so parallel edges never merge onto one point. Async; the engine is loaded
 * lazily. Falls back to the grid `layoutGraph` if the engine fails. Hidden nodes are simply not
 * in `work`, so their edges are gone too.
 */
export async function layoutGraphElk(work: readonly LhWork[]): Promise<Layout> {
  if (work.length === 0) return { nodes: [], edges: [], width: 0, height: 0 };
  try {
    const { default: ELK } = await import('elkjs/lib/elk.bundled.js');
    const edges = graphEdges(work);
    const layer = layoutGraph(work).nodes; // the grid's layer numbers (longest input chain)
    const layerOf = new Map(layer.map((n) => [n.id, n.layer]));
    const graph = {
      id: 'root',
      layoutOptions: ELK_OPTS,
      children: work.map((w) => ({
        id: w.workId,
        width: NODE_W,
        height: NODE_H,
        layoutOptions: { 'elk.portConstraints': 'FIXED_SIDE', 'elk.portAlignment.default': 'DISTRIBUTED' },
        ports: [
          ...edges.filter((e) => e.to === w.workId).map((e, i) => ({ id: `${w.workId}:in:${e.from}:${e.kind}:${i}`, layoutOptions: { 'elk.port.side': 'NORTH' } })),
          ...edges.filter((e) => e.from === w.workId).map((e, i) => ({ id: `${w.workId}:out:${e.to}:${e.kind}:${i}`, layoutOptions: { 'elk.port.side': 'SOUTH' } })),
        ],
      })),
      edges: edges.map((e, i) => ({
        id: `e${i}`,
        sources: [`${e.from}:out:${e.to}:${e.kind}:${edges.filter((x) => x.from === e.from).indexOf(e)}`],
        targets: [`${e.to}:in:${e.from}:${e.kind}:${edges.filter((x) => x.to === e.to).indexOf(e)}`],
      })),
    };
    const out = (await new ELK().layout(graph as never)) as ElkOut;
    const nodes: LaidNode[] = (out.children ?? []).map((c) => ({ id: c.id, x: c.x ?? 0, y: c.y ?? 0, layer: layerOf.get(c.id) ?? 0 }));
    const laid: LaidEdge[] = edges.map((e, i) => {
      const sec = out.edges?.find((x) => x.id === `e${i}`)?.sections?.[0];
      const points = sec ? [sec.startPoint, ...(sec.bendPoints ?? []), sec.endPoint] : [];
      return { ...e, points };
    });
    // An edge ELK could not route is dropped rather than drawn to nowhere.
    return { nodes, edges: laid.filter((e) => e.points.length >= 2), width: out.width ?? 0, height: out.height ?? 0 };
  } catch {
    return layoutGraph(work, 2);
  }
}
