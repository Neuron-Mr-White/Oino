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
const MEM_LIMIT = 64;
const DISK_LIMIT = 20;
const DISK_MAX_TURNS = 24;
const DISK_STR_CAP = 6_200;

const memCache = new Map<string, Turn[]>();
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

function persistToDisk(sessionKey: string, turns: readonly Turn[]): void {
  if (typeof localStorage === 'undefined' || !turns.length) return;
  try {
    const slice = turns.slice(-DISK_MAX_TURNS).map(slimTurnForDisk);
    const payload = JSON.stringify(slice);
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

/** Last-known turns for `sessionKey` (`hostId/sessionId`), or `undefined` when uncached. */
export function readTimelineSnapshot(sessionKey: string): Turn[] | undefined {
  const hit = memCache.get(sessionKey);
  if (hit && hit.length) {
    memCache.delete(sessionKey);
    memCache.set(sessionKey, hit);
    return hit;
  }
  if (typeof localStorage === 'undefined') return undefined;
  try {
    const raw = localStorage.getItem(STORAGE_ITEM_PREFIX + sessionKey);
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed) || !parsed.length) return undefined;
    const turns = parsed as Turn[];
    memCache.set(sessionKey, turns);
    return turns;
  } catch {
    return undefined;
  }
}

/** Records settled/current turns for `sessionKey`; persists to disk off the hot path. */
export function writeTimelineSnapshot(sessionKey: string, turns: readonly Turn[]): void {
  if (!sessionKey || !turns.length) return;
  const settledCopy = turns.map((t) => (t.live ? slimTurnForDisk(t) : t));
  memCache.delete(sessionKey);
  memCache.set(sessionKey, settledCopy);
  while (memCache.size > MEM_LIMIT) {
    const oldest = memCache.keys().next();
    if (oldest.done) break;
    memCache.delete(oldest.value);
  }
  const prev = pendingWrites.get(sessionKey);
  if (prev !== undefined) clearTimeout(prev);
  pendingWrites.set(
    sessionKey,
    setTimeout(() => {
      pendingWrites.delete(sessionKey);
      persistToDisk(sessionKey, settledCopy);
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
