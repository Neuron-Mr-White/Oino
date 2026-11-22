/**
 * Per-chat cache in front of the bridge's `paths_stat` (UNI-204): every
 * rendered block asks "which of these candidate paths exist on the PC?";
 * asks made in the same tick are merged into one request (≤ 200 paths
 * each), answers are cached per chat (existing for 5 min, missing for
 * 30 s so a file the agent creates a moment later still lights up on the
 * next render). A `null` answer (offline, timeout, an older pi without
 * `paths_stat`) isn't cached — those paths just stay plain.
 */
import type { PathStatItem } from './store';

export type StatChat = { statPaths?: (paths: string[]) => Promise<PathStatItem[] | null> };

/**
 * Everything file/image ref plumbing (`fileRefOpen`, `fileRefGestures`,
 * `pathTokens`, `FileRefText`, `MdBlock`, `ImageChip`) actually reads off
 * "a chat": the v1 `Chat` satisfies it, and the v2 timeline passes a
 * `SessionHandle`-backed adapter (WP-A6) so bubbles and markdown keep
 * verified refs without dragging the old store in.
 */
export type RefChat = StatChat & {
  hostId: string;
  session?: string;
  state?: { conn?: string };
  /** Session cwd (project folder) — ~/ display, mention text, relative opens. */
  cwd?: () => string;
  /** `ImageRef.mediaRef` bytes → data URL (inline `data` never calls this). */
  media?: (mediaRef: string, mime: string) => Promise<{ mime: string; dataUrl: string }>;
  /** Allow-lists a path for `blob_get` (previewing a text-seen path). */
  shareFile?: (path: string) => Promise<unknown>;
};

const BATCH_MAX = 200;
const HIT_TTL_MS = 5 * 60_000;
const MISS_TTL_MS = 30_000;

type Entry = { item: PathStatItem; at: number };
type Cache = {
  done: Map<string, Entry>;
  inflight: Map<string, Promise<PathStatItem | undefined>>;
  queue: Map<string, Array<(item: PathStatItem | undefined) => void>>;
  scheduled: boolean;
  /** The last ask got no answer (`null`: an older pi without `paths_stat`, offline, timeout). */
  unavailable: boolean;
};

const caches = new WeakMap<object, Cache>();

function cacheFor(chat: object): Cache {
  let c = caches.get(chat);
  if (!c) {
    c = { done: new Map(), inflight: new Map(), queue: new Map(), scheduled: false, unavailable: false };
    caches.set(chat, c);
  }
  return c;
}

function flush(chat: StatChat, c: Cache): void {
  c.scheduled = false;
  const all = [...c.queue.entries()];
  c.queue.clear();
  for (let i = 0; i < all.length; i += BATCH_MAX) {
    const batch = all.slice(i, i + BATCH_MAX);
    const paths = batch.map(([p]) => p);
    const ask = chat.statPaths ? chat.statPaths(paths).catch(() => null) : Promise.resolve(null);
    void ask.then((items) => {
      const byPath = new Map((items ?? []).map((it) => [it.path, it]));
      const now = Date.now();
      c.unavailable = items === null;
      for (const [path, waiters] of batch) {
        const item = items ? byPath.get(path) : undefined;
        if (item) c.done.set(path, { item, at: now });
        c.inflight.delete(path);
        for (const w of waiters) w(item);
      }
    });
  }
}

/** The cached answer for `path`, if still fresh (synchronous; for renders that can't wait). */
export function cachedStat(chat: object, path: string, now = Date.now()): PathStatItem | undefined {
  const e = caches.get(chat)?.done.get(path);
  if (!e) return undefined;
  const ttl = e.item.kind === 'missing' ? MISS_TTL_MS : HIT_TTL_MS;
  return now - e.at < ttl ? e.item : undefined;
}

/** Stats `paths` (deduped, batched with every other ask in this tick). Resolves a map of the
 * answers that came back; paths with no answer are absent. */
export function lookupPaths(chat: StatChat, paths: readonly string[]): Promise<Map<string, PathStatItem>> {
  const c = cacheFor(chat);
  const out = new Map<string, PathStatItem>();
  const waits: Array<Promise<void>> = [];
  for (const path of new Set(paths)) {
    const hit = cachedStat(chat, path);
    if (hit) {
      out.set(path, hit);
      continue;
    }
    let p = c.inflight.get(path);
    if (!p) {
      p = new Promise<PathStatItem | undefined>((resolve) => {
        const list = c.queue.get(path) ?? [];
        list.push(resolve);
        c.queue.set(path, list);
      });
      c.inflight.set(path, p);
      if (!c.scheduled) {
        c.scheduled = true;
        queueMicrotask(() => flush(chat, c));
      }
    }
    waits.push(
      p.then((item) => {
        if (item) out.set(path, item);
      }),
    );
  }
  return Promise.all(waits).then(() => out);
}

/**
 * True while this chat's bridge can't verify paths (the last `paths_stat`
 * came back `null`: a pi from before UNI-204 answers "unknown type", or
 * the ask timed out). Callers then fall back to the pre-UNI-204 behaviour
 * for the shapes that were linked before (tool-row subjects, absolute
 * paths) instead of leaving nothing tappable. Cleared by the next real
 * answer.
 */
export function pathStatUnavailable(chat: object): boolean {
  return caches.get(chat)?.unavailable ?? false;
}

/** Test hook. */
export function resetPathStatCache(chat: object): void {
  caches.delete(chat);
}
