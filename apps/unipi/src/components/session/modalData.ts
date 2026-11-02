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
  contextUsage?: ContextUsage | null;
};

export type RunningBash = { toolCallId: string; command: string; startedAt: number; pid?: number };

export type AgentInfo = { id: string; parentId?: string; label: string; owner: string; role?: string; state: 'running' | 'idle' | 'hibernated' | 'closed'; depth: number };

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

/** `$1.23` / `$0.004` — small costs keep decimals so they don't read as $0.00. */
export function formatUsd(n: number | undefined): string {
  if (!n) return '$0.00';
  return n < 0.01 ? `$${n.toFixed(4)}` : `$${n.toFixed(2)}`;
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
