/**
 * Session modal data helpers (WP-A17, D-7): the wire types and fetchers the
 * three tabs share over the session stream (`SessionHandle.cmd`). The shapes
 * are oino's: `ContextUsage` / `SessionStats` (camelCase, core
 * `session/stats.rs`), `RunningBash` (`session/feeds.rs`), `AgentInfo`
 * (`agents/feed.rs`), `ExtensionInfo` (`session/feeds.rs`), and the
 * `_oino/service_call` replies. Values are narrowed defensively (D-U14: the
 * hot path never zod-parses; these are cold one-shots, still kept lenient).
 */
import type { SessionHandle } from '../../lib/session/store';
import type { CommandInfo } from '../../lib/session/types';
import { parseSurfaces, type Surface } from '../../lib/surfaces';

export type { CommandInfo };

export type ContextUsage = { tokens: number | null; contextWindow: number; percent: number | null };

export type TokenTotals = { input: number; output: number; cacheRead: number; cacheWrite: number; total: number };

export type SessionStats = {
  sessionFile?: string;
  sessionId: string;
  userMessages: number;
  assistantMessages: number;
  toolCalls: number;
  toolResults: number;
  totalMessages: number;
  tokens: TokenTotals;
  cost: number;
  /** Tokens (child runs included) spent on models whose provider reports no price: `cost` leaves them out. */
  unpricedTokens: number;
  contextUsage?: ContextUsage | null;
};

export type RunningBash = { toolCallId: string; command: string; startedAt: number; pid?: number };

export type AgentInfo = { id: string; parentId?: string; label: string; owner: string; role?: string; state: 'running' | 'idle' | 'hibernated' | 'closed'; depth: number; sessionFile?: string };

/** A subagent run's structured status (`unipi.subagents` `Status`, wire lowercase) — never the TUI badge text. */
export type SubagentStatus = 'running' | 'completed' | 'failed' | 'cancelled';

/** Usage totals of one run (`child::Usage`, camelCase). */
export type RunUsage = { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };

/** One subagent run record (`unipi.subagents` `Record`: the `unipi:subagents:list` topic / `subagents.list`). */
export type SubagentRun = {
  id: string;
  title: string;
  profile: string;
  model: string;
  status: SubagentStatus;
  background: boolean;
  startedAt: number;
  endedAt?: number;
  toolCalls: number;
  usage?: RunUsage;
  error?: string;
  sessionFile?: string;
  depth: number;
};

export type ExtensionInfo = { id: string; kind?: string; enabled: boolean; loaded: boolean; path?: string };

/** One work-tray tab (unipi-kit `TrayTab`, camelCase). */
export type TrayTab = {
  id: string;
  label: string;
  shortLabel?: string;
  order: number;
  counts: { total: number; running: number };
  visible?: boolean;
  body?: unknown;
  owner?: string;
  /** The clients that list the tab (absent = every client; `lib/surfaces`). */
  surfaces?: Surface[];
};

const num = (v: unknown, or = 0): number => (typeof v === 'number' && Number.isFinite(v) ? v : or);
const optNum = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown, or = ''): string => (typeof v === 'string' ? v : or);

/** A resource source that changes with the stream's connection state: `down` while
 * connecting/reconnecting (the fetcher skips fetching), `live` on every (re)connection —
 * a fetch that raced `connecting` is superseded instead of swallowed. */
export const liveSource = (s: () => SessionHandle) => (): 'live' | 'down' =>
  s().state.conn === 'live' ? 'live' : 'down';

/** Tool calls the transcript itself shows (assistant `toolCall` parts, failed ones too) — PH-40:
 *  the host's count can miss calls whose result never landed, so the sheet shows the larger. */
export function transcriptToolCalls(entries: ReadonlyArray<{ message?: { role?: string; content?: unknown } }>): number {
  let n = 0;
  for (const e of entries) {
    const content = e.message?.role === 'assistant' ? e.message.content : undefined;
    if (Array.isArray(content)) for (const part of content) if ((part as { type?: unknown })?.type === 'toolCall') n += 1;
  }
  return n;
}

export function parseContextUsage(v: unknown): ContextUsage | undefined {
  if (!v || typeof v !== 'object') return undefined;
  const o = v as Record<string, unknown>;
  return { tokens: optNum(o.tokens), contextWindow: num(o.contextWindow), percent: optNum(o.percent) };
}

export function parseSessionStats(v: unknown): SessionStats | undefined {
  if (!v || typeof v !== 'object') return undefined;
  const o = v as Record<string, unknown>;
  const t = (o.tokens ?? {}) as Record<string, unknown>;
  return {
    sessionFile: typeof o.sessionFile === 'string' ? o.sessionFile : undefined,
    sessionId: str(o.sessionId),
    userMessages: num(o.userMessages),
    assistantMessages: num(o.assistantMessages),
    toolCalls: num(o.toolCalls),
    toolResults: num(o.toolResults),
    totalMessages: num(o.totalMessages),
    tokens: { input: num(t.input), output: num(t.output), cacheRead: num(t.cacheRead), cacheWrite: num(t.cacheWrite), total: num(t.total) },
    cost: num(o.cost),
    unpricedTokens: num(o.unpricedTokens),
    contextUsage: parseContextUsage(o.contextUsage),
  };
}

export function parseRunningBash(v: unknown): RunningBash[] {
  const calls = (v as { calls?: unknown } | undefined)?.calls;
  if (!Array.isArray(calls)) return [];
  return calls.flatMap((c) => {
    if (!c || typeof c !== 'object') return [];
    const o = c as Record<string, unknown>;
    const id = str(o.toolCallId);
    return id ? [{ toolCallId: id, command: str(o.command), startedAt: num(o.startedAt), pid: typeof o.pid === 'number' ? o.pid : undefined }] : [];
  });
}

const STATES: AgentInfo['state'][] = ['running', 'idle', 'hibernated', 'closed'];

export function parseAgents(v: unknown): AgentInfo[] {
  const agents = (v as { agents?: unknown } | undefined)?.agents;
  if (!Array.isArray(agents)) return [];
  return agents.flatMap((a) => {
    if (!a || typeof a !== 'object') return [];
    const o = a as Record<string, unknown>;
    const id = str(o.id);
    const state = str(o.state).toLowerCase() as AgentInfo['state'];
    return id ? [{
      id,
      parentId: typeof o.parentId === 'string' ? o.parentId : undefined,
      label: str(o.label, 'Agent'),
      owner: str(o.owner),
      role: typeof o.role === 'string' ? o.role : undefined,
      state: STATES.includes(state) ? state : 'running',
      depth: num(o.depth),
      ...(typeof o.sessionFile === 'string' && o.sessionFile ? { sessionFile: o.sessionFile } : {}),
    }] : [];
  });
}

export function parseExtensions(v: unknown): ExtensionInfo[] {
  const list = (v as { extensions?: unknown } | undefined)?.extensions;
  if (!Array.isArray(list)) return [];
  return list.flatMap((e) => {
    if (!e || typeof e !== 'object') return [];
    const o = e as Record<string, unknown>;
    const id = str(o.id);
    return id ? [{ id, kind: typeof o.kind === 'string' ? o.kind : undefined, enabled: o.enabled !== false, loaded: o.loaded === true, path: typeof o.path === 'string' ? o.path : undefined }] : [];
  });
}

export function parseTrayTabs(v: unknown): TrayTab[] {
  const list = Array.isArray(v) ? v : [];
  return list.flatMap((t) => {
    if (!t || typeof t !== 'object') return [];
    const o = t as Record<string, unknown>;
    const id = str(t.id);
    if (!id) return [];
    const c = (o.counts ?? {}) as Record<string, unknown>;
    return [{
      id,
      label: str(o.label, id),
      shortLabel: typeof o.shortLabel === 'string' ? o.shortLabel : undefined,
      order: num(o.order),
      counts: { total: num(c.total), running: num(c.running) },
      visible: o.visible !== false,
      body: o.body,
      owner: typeof o.owner === 'string' ? o.owner : undefined,
      ...(parseSurfaces(o.surfaces) ? { surfaces: parseSurfaces(o.surfaces) } : {}),
    }];
  });
}

const RUN_STATUSES: SubagentStatus[] = ['running', 'completed', 'failed', 'cancelled'];

/**
 * Subagent run records: the topic value (`{records:[…]}`) or the `subagents.list` reply (a
 * list). `undefined` when the value is neither (no topic yet / an older extension). The status
 * comes from the record's structured `status`, never from a TUI label.
 */
export function parseSubagentRuns(v: unknown): SubagentRun[] | undefined {
  const list = Array.isArray(v) ? v : (v as { records?: unknown } | undefined | null)?.records;
  if (!Array.isArray(list)) return undefined;
  return list.flatMap((r) => {
    if (!r || typeof r !== 'object') return [];
    const o = r as Record<string, unknown>;
    const id = str(o.id);
    if (!id) return [];
    const status = str(o.status).toLowerCase() as SubagentStatus;
    const u = o.usage && typeof o.usage === 'object' ? (o.usage as Record<string, unknown>) : undefined;
    const title = str(o.title).trim() || str(o.task).trim().slice(0, 80) || id;
    return [{
      id,
      title,
      profile: str(o.profile),
      model: str(o.model),
      status: RUN_STATUSES.includes(status) ? status : 'running',
      background: o.background === true,
      startedAt: num(o.startedAt),
      ...(typeof o.endedAt === 'number' && Number.isFinite(o.endedAt) ? { endedAt: o.endedAt } : {}),
      toolCalls: num(o.toolCalls),
      ...(u ? { usage: { input: num(u.input), output: num(u.output), cacheRead: num(u.cacheRead), cacheWrite: num(u.cacheWrite), cost: num(u.cost) } } : {}),
      ...(typeof o.error === 'string' && o.error ? { error: o.error } : {}),
      ...(typeof o.sessionFile === 'string' && o.sessionFile ? { sessionFile: o.sessionFile } : {}),
      depth: num(o.depth, 1),
    }];
  });
}

/** `_oino/service_call{target?, name, payload}` → the service's reply. */
export function serviceCall<T = unknown>(s: SessionHandle, name: string, payload?: Record<string, unknown>, target?: string): Promise<T> {
  return s.cmd<T>({ type: '_oino/service_call', ...(target ? { target } : {}), name, ...(payload ? { payload } : {}) });
}

/** `1.2k` / `845` — compact token counts (v1 `formatCount`). */
export function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

/** `$1.23` / `$0.0042` — small costs keep decimals so they don't read as $0.00; `—` when unknown
 *  (stats not loaded yet — never a made-up `$0.00`). */
export function formatUsd(n: number | undefined): string {
  if (n === undefined || !Number.isFinite(n)) return '—';
  if (!n) return '$0.00';
  return n < 0.01 ? `$${n.toFixed(4)}` : `$${n.toFixed(2)}`;
}

/**
 * The session cost as the Info / Context tabs show it ([status], owner 2026-11-04 "session cost
 * is 0"): child / subagent runs are in `cost` already (core rolls their usage into the lead
 * session). A provider that reports no price (cost 0 with tokens spent) reads `—` plus its token
 * count instead of `$0.00`; a partly priced session reads the priced sum with the unpriced
 * tokens beside it.
 */
export function sessionCostView(stats: Pick<SessionStats, 'cost' | 'tokens' | 'unpricedTokens'> | undefined): { text: string; sub?: string; title: string } {
  if (!stats) return { text: '—', title: 'Loading the session cost' };
  const unpriced = stats.unpricedTokens > 0 ? stats.unpricedTokens : stats.cost > 0 ? 0 : stats.tokens.total;
  if (stats.cost > 0) {
    const text = formatUsd(stats.cost);
    if (unpriced <= 0) return { text, title: `${text}, subagent runs included` };
    const sub = `+ ${formatCount(unpriced)} tokens unpriced`;
    return { text, sub, title: `${text} on priced models, subagent runs included; ${formatCount(unpriced)} more tokens ran on models without a price` };
  }
  if (unpriced > 0) {
    const sub = `${formatCount(unpriced)} tokens, no price`;
    return { text: '—', sub, title: `The provider reports no price for this session's models (${formatCount(unpriced)} tokens)` };
  }
  return { text: '$0.00', title: 'Nothing spent yet' };
}

/** `1m 3s` / `2h 5m` — compact durations for running rows. */
export function formatDurationMs(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

/** One model's share of this session (assistant replies on the loaded branch). */
export type SessionModelUse = { model: string; turns: number; input: number; output: number; cacheRead: number; cacheWrite: number; tokens: number; cost: number };

/**
 * This session's usage per model, from its own assistant messages: a reply whose model reports no
 * price (cost 0, tokens spent) keeps its tokens and `cost` 0 — the view shows `—`, never `$0`.
 * Largest first. Pure.
 */
export function sessionModelUsage(entries: ReadonlyArray<{ type: string; message?: { role?: string; model?: string; usage?: { input?: number; output?: number; cacheRead?: number; cacheWrite?: number; cost?: { total?: number } } } }>): SessionModelUse[] {
  const by = new Map<string, SessionModelUse>();
  for (const e of entries) {
    const m = e.type === 'message' ? e.message : undefined;
    if (m?.role !== 'assistant' || !m.usage) continue;
    const key = (m.model ?? 'unknown').replace(/^.*\//, '');
    const row = by.get(key) ?? { model: key, turns: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, tokens: 0, cost: 0 };
    const u = m.usage;
    row.turns += 1;
    row.input += u.input ?? 0;
    row.output += u.output ?? 0;
    row.cacheRead += u.cacheRead ?? 0;
    row.cacheWrite += u.cacheWrite ?? 0;
    row.tokens = row.input + row.output + row.cacheRead + row.cacheWrite;
    row.cost += u.cost?.total ?? 0;
    by.set(key, row);
  }
  return [...by.values()].sort((a, b) => b.tokens - a.tokens);
}

/** Wall-clock span of the session: first to last entry timestamp (ms), or undefined with < 2 stamps. Pure. */
export function sessionSpanMs(entries: ReadonlyArray<{ timestamp?: string }>): number | undefined {
  let lo = Infinity;
  let hi = -Infinity;
  for (const e of entries) {
    const t = e.timestamp ? Date.parse(e.timestamp) : NaN;
    if (Number.isFinite(t)) {
      lo = Math.min(lo, t);
      hi = Math.max(hi, t);
    }
  }
  return hi > lo ? hi - lo : undefined;
}
