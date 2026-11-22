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

/**
 * v2 (1.0.10 rc4): v1 holds snapshots that were taken while reference-only stubs were still unhydrated (a
 * stub-less chat that restored as authoritative after a relaunch, G3-1) and unslimmed tool arguments. They
 * are dropped, never read.
 */
const STORAGE_INDEX_KEY = 'unipi.tl.snap.index.v2';
const STORAGE_ITEM_PREFIX = 'unipi.tl.snap.item.v2:';
const LEGACY_PREFIXES = ['unipi.tl.snap.item.v1:', 'unipi.tl.snap.index.v1'];
/** One session's persisted snapshot never takes more than this many characters (an oversized chat is simply not persisted). */
const DISK_ITEM_MAX = 900_000;
/** Tool arguments of this size stay whole in the snapshot; bigger ones are cut like every other string. */
const ARGS_KEEP = 8_000;
/** In-memory ceiling for the whole cache, in (approximate) serialized bytes — not a session count. */
export const MEM_BYTE_LIMIT = 8 * 1024 * 1024;
/** One session never takes more than this share (an oversized chat is simply not cached in memory). */
const MEM_ENTRY_MAX = 2 * 1024 * 1024;
const DISK_LIMIT = 20;
const DISK_MAX_TURNS = 24;
const DISK_STR_CAP = 6_200;

/** Least recently used first. `bytes` = 0 while a write is still pending (the entry then holds the live array). */
const memCache = new Map<string, { turns: Turn[]; bytes: number }>();
const pendingWrites = new Map<string, ReturnType<typeof setTimeout>>();

/**
 * w5-ui (D10): a cached prefix ends at a paragraph (else line, else word) boundary, never mid-word — the warm-launch
 * view of a long answer is a clean excerpt, not a sentence cut in half that jumps when the live text lands.
 */
export function cutAtBoundary(s: string, cap: number): string {
  if (s.length <= cap) return s;
  const head = s.slice(0, cap);
  const floor = Math.floor(cap * 0.6);
  const para = head.lastIndexOf('\n\n');
  if (para >= floor) return head.slice(0, para);
  const line = head.lastIndexOf('\n');
  if (line >= floor) return head.slice(0, line);
  const word = head.search(/\s\S*$/);
  return word >= floor ? head.slice(0, word) : head;
}

const trimStr = (s: string | undefined, cap = DISK_STR_CAP): string | undefined =>
  s === undefined ? undefined : cutAtBoundary(s, cap);

function slimEntry(e: SessionEntry | undefined): SessionEntry | undefined {
  if (!e) return undefined;
  const msg = e.message;
  if (!msg) {
    const slim = { type: e.type, id: e.id, timestamp: e.timestamp, customType: e.customType } as SessionEntry;
    // A workspace move's marker names the folder from its details (small: `{cwd, previousCwd}`).
    if (e.customType === 'oino.cwd_change' && e.details !== undefined) slim.details = e.details;
    // [chat-events] an event row's summary needs its fields; each one is capped so the cache stays bounded.
    for (const k of EVENT_FIELDS) {
      const v = (e as Record<string, unknown>)[k];
      if (v === undefined || slim[k] !== undefined) continue;
      if (typeof v === 'string') slim[k] = trimStr(v, EVENT_STR_CAP);
      else if (typeof v === 'number' || typeof v === 'boolean') slim[k] = v;
      else if (smallJson(v)) slim[k] = v;
    }
    return slim;
  }
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
      // [chat-events] a `!cmd` row's command / exit / output head.
      ...(msg.role === 'bashExecution'
        ? {
            command: trimStr(msg.command as string | undefined, EVENT_STR_CAP),
            output: trimStr(msg.output as string | undefined, EVENT_STR_CAP),
            exitCode: msg.exitCode,
            cancelled: msg.cancelled,
            ...(msg.excludeFromContext !== undefined ? { excludeFromContext: msg.excludeFromContext } : {}),
          }
        : {}),
    },
  } as SessionEntry;
}

/** Top-level fields an event row reads (custom content/details/data, branch summary, model change). */
const EVENT_FIELDS = ['content', 'display', 'details', 'data', 'summary', 'modelId', 'provider', 'thinkingLevel', 'tokensBefore'] as const;
const EVENT_STR_CAP = 2_000;
const EVENT_JSON_CAP = 2_000;
const smallJson = (v: unknown): boolean => {
  try {
    return JSON.stringify(v).length <= EVENT_JSON_CAP;
  } catch {
    return false;
  }
};

/** Child step rows a cached subagent card keeps (the live card caps at 200 too). */
const AGENT_STEPS_CACHED = 50;

/** A child step with only what its mini-timeline row shows (no output, clipped argument). */
function slimStep(e: SessionEntry): SessionEntry {
  const d = (e.data ?? {}) as Record<string, unknown>;
  const pick: Record<string, unknown> = {};
  for (const k of ['kind', 'name', 'isError', 'durationMs', 'agentId', 'id', 'title', 'profile']) if (d[k] !== undefined) pick[k] = d[k];
  if (typeof d.arg === 'string') pick.arg = d.arg.slice(0, 300);
  if (typeof d.text === 'string') pick.text = d.text.slice(0, 300);
  return { type: e.type, id: e.id, timestamp: e.timestamp, customType: e.customType, data: pick } as SessionEntry;
}

/** A tool call's arguments with every long string cut (a 1.4 MB `command` must not fill the persisted snapshot). */
function slimValue(v: unknown, depth = 0): unknown {
  if (typeof v === 'string') return trimStr(v);
  if (depth >= 4 || v === null || typeof v !== 'object') return v;
  if (Array.isArray(v)) return v.slice(0, 20).map((x) => slimValue(x, depth + 1));
  const out: Record<string, unknown> = {};
  for (const [k, x] of Object.entries(v)) out[k] = slimValue(x, depth + 1);
  return out;
}

function slimArgs(args: Record<string, unknown>): Record<string, unknown> {
  try {
    if (JSON.stringify(args).length <= ARGS_KEEP) return args;
  } catch {
    // unserialisable: cut below
  }
  return slimValue(args) as Record<string, unknown>;
}

function slimTool(t: ToolRun): ToolRun {
  return {
    ...t,
    args: slimArgs(t.args),
    state: t.state === 'running' || t.state === 'preparing' ? 'stopped' : t.state,
    output: trimStr(t.output),
    live: undefined,
  };
}

function slimItem(w: WorkItem): WorkItem {
  if (w.kind === 'tool') return { ...w, tool: slimTool(w.tool) };
  if (w.kind === 'prose') return { ...w, text: trimStr(w.text) ?? '', live: false, ...(w.text.length > DISK_STR_CAP ? { cut: true } : {}) };
  if (w.kind === 'thinking') return { ...w, text: trimStr(w.text) ?? '', live: false };
  if (w.kind === 'step' || w.kind === 'card' || w.kind === 'compaction') return { ...w, entry: slimEntry(w.entry)! };
  if (w.kind === 'agent') {
    const { current: _current, ...run } = w.run;
    return {
      ...w,
      ...(w.tool ? { tool: slimTool(w.tool) } : {}),
      run: { ...run, report: trimStr(run.report), steps: run.steps.slice(-AGENT_STEPS_CACHED).map(slimStep) },
    };
  }
  return w;
}

function slimNode(n: WorkNode): WorkNode {
  if (n.kind === 'agents') {
    const items = n.items.map(slimItem);
    return { ...n, items, runs: items.flatMap((i) => (i.kind === 'agent' ? [i.run] : [])) };
  }
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
    ...(s.before ? { before: s.before.map(slimNode) } : {}),
    ...(s.after ? { after: s.after.map(slimNode) } : {}),
    // [ask-user] the inline question card keeps its state (a waiting question stays answerable).
    ...(s.ask ? { ask: { ...s.ask, output: trimStr(s.ask.output), live: undefined } } : {}),
    answer: s.answer ? { ...s.answer, text: trimStr(s.answer.text, 24_000) ?? '', live: false, ...(s.answer.text.length > 24_000 ? { cut: true } : {}) } : undefined,
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

let legacyPurged = false;
/** Once per page load: the v1 snapshots go (see `STORAGE_ITEM_PREFIX`). */
function purgeLegacy(): void {
  if (legacyPurged || typeof localStorage === 'undefined') return;
  legacyPurged = true;
  try {
    const stale: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && LEGACY_PREFIXES.some((p) => k.startsWith(p))) stale.push(k);
    }
    for (const k of stale) localStorage.removeItem(k);
  } catch {
    // ignore
  }
}

/** Drops the persisted snapshot of `sessionKey`: a snapshot that could not be refreshed must never restore as the current one. */
function dropFromDisk(sessionKey: string): void {
  try {
    localStorage.removeItem(STORAGE_ITEM_PREFIX + sessionKey);
    localStorage.setItem(STORAGE_INDEX_KEY, JSON.stringify(readDiskIndex().filter((k) => k !== sessionKey)));
  } catch {
    // ignore
  }
}

function persistToDisk(sessionKey: string, payload: string): void {
  if (typeof localStorage === 'undefined') return;
  purgeLegacy();
  if (payload.length > DISK_ITEM_MAX) {
    dropFromDisk(sessionKey);
    return;
  }
  try {
    localStorage.setItem(STORAGE_ITEM_PREFIX + sessionKey, payload);
    const next = [...readDiskIndex().filter((k) => k !== sessionKey), sessionKey];
    while (next.length > DISK_LIMIT) {
      const evicted = next.shift();
      if (evicted) localStorage.removeItem(STORAGE_ITEM_PREFIX + evicted);
    }
    localStorage.setItem(STORAGE_INDEX_KEY, JSON.stringify(next));
  } catch {
    // Quota or private mode: in-memory cache still serves the session — and the older persisted copy goes,
    // so a relaunch shows nothing rather than an out-of-date snapshot.
    dropFromDisk(sessionKey);
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
  purgeLegacy();
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
 * Records settled/current turns for `sessionKey`. `complete: false` (reference-only stubs still unhydrated: the
 * turns lack their content) records nothing — a snapshot is only ever taken from hydrated content. The write is debounced: until it
 * lands the entry points at the live array (no copy on the streaming hot path);
 * then the cache keeps only the slimmed copy (no base64 images, capped strings, last
 * `DISK_MAX_TURNS` turns), measures it, and evicts by total bytes.
 */
export function writeTimelineSnapshot(sessionKey: string, turns: readonly Turn[], complete = true): void {
  if (!sessionKey || !turns.length || !complete) return;
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
