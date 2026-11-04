/**
 * [chat-load] Turn-aligned history chunks (design: oino-internal
 * docs/dev/v2/design/chat-render-pipeline.md §New.a).
 *
 * The host pages a branch by ENTRIES (`get_entries{before, limit}`, aligned forward to the
 * first user message of the page — which may be a steer inside a turn, or absent when the
 * whole page is one giant turn). The timeline must only ever see WHOLE turns, so the store
 * assembles a chunk from as many host pages as it takes to reach a real turn start and
 * commits the chunk from that start in one mutation. Entries older than the start (the
 * newer part of the turn before it) are dropped and fetched again, whole, with the next
 * chunk — the next cursor is the chunk's first entry.
 *
 * A turn start here is exactly what the turns index (`turns.ts` `scanStarts`) starts a
 * unit at: a user message whose previous message is not a tool result / `toolUse` stop
 * (that would be a steer), or a compaction after an ended run. A start is only known when
 * its previous message is loaded too, so the first entry of a page never counts unless the
 * branch begins there.
 */
import type { AgentMessage, SessionEntry } from './types';

// The two predicates below MUST match `turns.ts` (`isSteer`, `isMarkerCompaction`); the
// chunks test cross-checks every boundary against the turns index.
const TERMINAL = new Set(['stop', 'aborted', 'error', 'length']);
function isSteer(prev: AgentMessage | undefined): boolean {
  if (!prev) return false;
  if (prev.role === 'toolResult') return true;
  return prev.role === 'assistant' && prev.stopReason === 'toolUse';
}
function isMarkerCompaction(prev: AgentMessage | undefined): boolean {
  if (!prev) return true;
  if (prev.role === 'assistant') return TERMINAL.has(String(prev.stopReason ?? ''));
  return prev.role === 'user';
}

/** Entries per host page while assembling a chunk (the host clamps to 500 and its frame budget). */
export const CHUNK_PAGE = 200;
/** Host pages per chunk at most. */
export const CHUNK_MAX_PAGES = 10;
/**
 * A chunk stops growing at this many entries even without a turn start (one turn longer
 * than this — a very long graph run — is the only case that commits a partial turn).
 */
export const CHUNK_MAX_ENTRIES = CHUNK_PAGE * CHUNK_MAX_PAGES;

const isMsg = (e: SessionEntry | undefined): e is SessionEntry & { message: AgentMessage } => e?.type === 'message' && !!e.message;

/**
 * Is `entries[i]` the first entry of a turn unit? `atBranchStart`: `entries[0]` is the
 * first entry of the branch (nothing older exists), so it starts a unit whatever it is.
 * Unknown (the previous message is not in `entries`) counts as no.
 */
export function isTurnStart(entries: readonly SessionEntry[], i: number, atBranchStart: boolean): boolean {
  const e = entries[i];
  if (!e) return false;
  if (i === 0) return atBranchStart;
  let prev: AgentMessage | undefined;
  let found = false;
  for (let j = i - 1; j >= 0; j--) {
    const p = entries[j];
    if (isMsg(p)) {
      prev = p.message;
      found = true;
      break;
    }
  }
  if (!found && !atBranchStart) return false;
  if (isMsg(e) && e.message.role === 'user') return !isSteer(prev);
  if (e.type === 'compaction') return isMarkerCompaction(prev);
  return false;
}

/** Index of the OLDEST turn start in `entries` (−1 = none known). */
export function oldestTurnStart(entries: readonly SessionEntry[], atBranchStart: boolean): number {
  for (let i = 0; i < entries.length; i++) if (isTurnStart(entries, i, atBranchStart)) return i;
  return -1;
}

/** Index of the turn start a trim may cut at: the newest start at or before `at` (−1 = none). */
export function turnStartAtOrBefore(entries: readonly SessionEntry[], at: number): number {
  for (let i = Math.min(at, entries.length - 1); i > 0; i--) if (isTurnStart(entries, i, false)) return i;
  return -1;
}

/** One host page (`get_entries{before}` reply). */
export type HostPage = { entries?: readonly SessionEntry[]; total?: number; hasMore?: boolean; cursor?: string | null };

/** What a chunk commits: whole turns, root-most first, plus the history fields. */
export type Chunk = { entries: SessionEntry[]; total?: number; hasMore: boolean; cursor?: string; pages: number; partial: boolean };

/**
 * Assembles one chunk before `cursor`: fetches host pages until the buffer holds a known
 * turn start (or the branch start, or the caps), then cuts it at its OLDEST turn start.
 * `known` filters entries already loaded. Never returns a split turn unless one turn
 * alone exceeds `CHUNK_MAX_ENTRIES` (`partial: true`).
 */
export async function assembleChunk(cursor: string, fetch: (before: string, limit: number) => Promise<HostPage | undefined>, known: (id: string) => boolean): Promise<Chunk> {
  let buf: SessionEntry[] = [];
  let before = cursor;
  let hasMore = true;
  let total: number | undefined;
  let pages = 0;
  const seen = new Set<string>();
  while (pages < CHUNK_MAX_PAGES) {
    const page = (await fetch(before, CHUNK_PAGE)) ?? {};
    pages++;
    if (typeof page.total === 'number') total = page.total;
    const older = (page.entries ?? []).filter((e) => !!e && typeof e.id === 'string' && !known(e.id) && !seen.has(e.id));
    for (const e of older) seen.add(e.id);
    buf = [...older, ...buf];
    hasMore = page.hasMore === true && older.length > 0;
    const next = page.cursor ?? older[0]?.id;
    if (!hasMore || !next) {
      hasMore = false;
      break;
    }
    before = next;
    if (oldestTurnStart(buf, false) >= 0 || buf.length >= CHUNK_MAX_ENTRIES) break;
  }
  if (!hasMore) return { entries: buf, total, hasMore: false, pages, partial: false };
  const start = oldestTurnStart(buf, false);
  if (start < 0) return { entries: buf, total, hasMore: true, cursor: buf[0]?.id ?? before, pages, partial: true };
  const entries = buf.slice(start);
  return { entries, total, hasMore: true, cursor: entries[0]!.id, pages, partial: false };
}
