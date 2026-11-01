/**
 * Timeline turn-snapshot cache: renders the last-known settled turns of a
 * session immediately (0 ms) while `session_open` connects, surviving both
 * the session store's 8-handle LRU eviction (in-memory LRU of 64 sessions)
 * and full app restarts (bounded `localStorage` ring of recent sessions).
 *
 * Once the live stream's snapshot arrives (`session.turns().length > 0` or
 * `conn === 'live'`), the timeline switches seamlessly to live turns and
 * refreshes the cached snapshot in the background.
 */
import type { Segment, ToolRun, Turn, WorkItem, WorkNode } from '../session/turns';
import type { SessionEntry } from '../session/types';

const STORAGE_INDEX_KEY = 'unipi.tl.snap.index.v1';
const STORAGE_ITEM_PREFIX = 'unipi.tl.snap.item.v1:';
/** In-memory ceiling for the whole cache, in (approximate) serialized bytes — not a session count. */
export const MEM_BYTE_LIMIT = 32 * 1024 * 1024;
/** One session never takes more than this share (an oversized chat is simply not cached in memory). */
const MEM_ENTRY_MAX = 8 * 1024 * 1024;
const DISK_LIMIT = 20;
const DISK_MAX_TURNS = 24;
const DISK_STR_CAP = 6_200;

/** Least recently used first. `bytes` = 0 while a write is still pending (the entry then holds the live array). */
const memCache = new Map<string, { turns: Turn[]; bytes: number }>();
const pendingWrites = new Map<string, ReturnType<typeof setTimeout>>();

const trimStr = (s: string | undefined, cap = DISK_STR_CAP): string | undefined =>
  s === undefined ? undefined : s.length > cap ? s.slice(0, cap) : s;

function slimEntry(e: SessionEntry | undefined): SessionEntry | undefined {
  if (!e) return undefined;
  const msg = e.message;
  if (!msg) return { type: e.type, id: e.id, timestamp: e.timestamp, customType: e.customType } as SessionEntry;
  const content = Array.isArray(msg.content)
    ? msg.content.map((p) => {
        if (p.type === 'image') {
          // Strip inline base64 from localStorage cache so images never blow quota.
          const { data: _data, ...rest } = p as Record<string, unknown>;
          return rest;
        }
        if (typeof (p as { text?: unknown }).text === 'string') {
          return { ...p, text: trimStr((p as { text?: string }).text) };
        }
        return p;
      })
    : typeof msg.content === 'string'
      ? trimStr(msg.content)
      : msg.content;
  return {
    type: e.type,
    id: e.id,
    timestamp: e.timestamp,
    customType: e.customType,
    message: {
      role: msg.role,
      content,
      timestamp: msg.timestamp,
      stopReason: msg.stopReason,
      ...(msg.unipiHarness ? { unipiHarness: msg.unipiHarness } : {}),
    },
  } as SessionEntry;
}

function slimTool(t: ToolRun): ToolRun {
  return {
    ...t,
    state: t.state === 'running' || t.state === 'preparing' ? 'stopped' : t.state,
    output: trimStr(t.output),
    live: undefined,
  };
}

function slimItem(w: WorkItem): WorkItem {
  if (w.kind === 'tool') return { ...w, tool: slimTool(w.tool) };
  if (w.kind === 'thinking' || w.kind === 'prose') return { ...w, text: trimStr(w.text) ?? '', live: false };
  if (w.kind === 'step' || w.kind === 'card' || w.kind === 'compaction') return { ...w, entry: slimEntry(w.entry)! };
  return w;
}

function slimNode(n: WorkNode): WorkNode {
  if (n.kind === 'group') {
    return {
      ...n,
      items: n.items.map(slimItem),
      tools: n.tools.map(slimTool),
    };
  }
  return slimItem(n) as WorkNode;
}

function slimSegment(s: Segment): Segment {
  return {
    ...s,
    steer: slimEntry(s.steer),
    work: s.work.map(slimItem),
    nodes: s.nodes.map(slimNode),
    answer: s.answer ? { ...s.answer, text: trimStr(s.answer.text, 24_000) ?? '', live: false } : undefined,
    fold: s.fold
      ? {
          ...s.fold,
          state: s.fold.state === 'running' ? 'done' : s.fold.state,
          label: s.fold.state === 'running' ? 'Worked' : s.fold.label,
          forcedOpen: s.fold.state === 'running' ? false : s.fold.forcedOpen,
        }
      : undefined,
    live: false,
  };
}

function slimTurnForDisk(t: Turn): Turn {
  return {
    ...t,
    user: slimEntry(t.user),
    compaction: slimEntry(t.compaction),
    // Keep only the anchor entries `TurnRow` / `FindBar` inspect before live snapshot lands.
    entries: t.user ? [slimEntry(t.user)!] : [],
    segments: t.segments.map(slimSegment),
    live: false,
  };
}

function readDiskIndex(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_INDEX_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

function persistToDisk(sessionKey: string, payload: string): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(STORAGE_ITEM_PREFIX + sessionKey, payload);
    const next = [...readDiskIndex().filter((k) => k !== sessionKey), sessionKey];
    while (next.length > DISK_LIMIT) {
      const evicted = next.shift();
      if (evicted) localStorage.removeItem(STORAGE_ITEM_PREFIX + evicted);
    }
    localStorage.setItem(STORAGE_INDEX_KEY, JSON.stringify(next));
  } catch {
    // Quota or private mode: in-memory cache still serves the session.
  }
}

function memBytes(): number {
  let n = 0;
  for (const e of memCache.values()) n += e.bytes;
  return n;
}

/** Drops least-recently-used sessions until the cache fits `MEM_BYTE_LIMIT` (never the entry just written). */
function evictMem(keep?: string): void {
  let total = memBytes();
  for (const [k, e] of memCache) {
    if (total <= MEM_BYTE_LIMIT) break;
    if (k === keep) continue;
    total -= e.bytes;
    memCache.delete(k);
  }
}

/** Test seam: current in-memory size (approximate bytes) and session count. */
export function snapshotCacheStats(): { bytes: number; sessions: number } {
  return { bytes: memBytes(), sessions: memCache.size };
}

/** Last-known turns for `sessionKey` (`hostId/sessionId`), or `undefined` when uncached. */
export function readTimelineSnapshot(sessionKey: string): Turn[] | undefined {
  const hit = memCache.get(sessionKey);
  if (hit && hit.turns.length) {
    memCache.delete(sessionKey);
    memCache.set(sessionKey, hit);
    return hit.turns;
  }
  if (typeof localStorage === 'undefined') return undefined;
  try {
    const raw = localStorage.getItem(STORAGE_ITEM_PREFIX + sessionKey);
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed) || !parsed.length) return undefined;
    const turns = parsed as Turn[];
    memCache.set(sessionKey, { turns, bytes: raw.length * 2 });
    evictMem(sessionKey);
    return turns;
  } catch {
    return undefined;
  }
}

/**
 * Records settled/current turns for `sessionKey`. The write is debounced: until it
 * lands the entry points at the live array (no copy on the streaming hot path);
 * then the cache keeps only the slimmed copy (no base64 images, capped strings, last
 * `DISK_MAX_TURNS` turns), measures it, and evicts by total bytes.
 */
export function writeTimelineSnapshot(sessionKey: string, turns: readonly Turn[]): void {
  if (!sessionKey || !turns.length) return;
  const prevEntry = memCache.get(sessionKey);
  memCache.delete(sessionKey);
  memCache.set(sessionKey, { turns: turns as Turn[], bytes: prevEntry?.bytes ?? 0 });
  const prev = pendingWrites.get(sessionKey);
  if (prev !== undefined) clearTimeout(prev);
  pendingWrites.set(
    sessionKey,
    setTimeout(() => {
      pendingWrites.delete(sessionKey);
      const slim = turns.slice(-DISK_MAX_TURNS).map(slimTurnForDisk);
      let payload: string | undefined;
      try {
        payload = JSON.stringify(slim);
      } catch {
        payload = undefined;
      }
      const bytes = (payload?.length ?? 0) * 2;
      if (!payload || bytes > MEM_ENTRY_MAX) memCache.delete(sessionKey);
      else if (memCache.has(sessionKey)) memCache.set(sessionKey, { turns: slim, bytes });
      evictMem(sessionKey);
      if (payload) persistToDisk(sessionKey, payload);
    }, 250),
  );
}

/** Test seam: clears both in-memory and persisted timeline snapshots. */
export function clearTimelineSnapshots(): void {
  for (const t of pendingWrites.values()) clearTimeout(t);
  pendingWrites.clear();
  memCache.clear();
  if (typeof localStorage === 'undefined') return;
  try {
    for (const k of readDiskIndex()) localStorage.removeItem(STORAGE_ITEM_PREFIX + k);
    localStorage.removeItem(STORAGE_INDEX_KEY);
  } catch {
    // ignore
  }
}
