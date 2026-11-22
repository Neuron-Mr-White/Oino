/**
 * Full messages (hotfix 1.0.9): the host cuts any message text over a budget when a chat opens
 * from a snapshot/page (`sessions/bound.rs`: `message.truncated = true` + a trailing note) to keep
 * frames small. Tool bodies pull the full result on expand (`useFullOutput`); prose — assistant /
 * user text, thinking, custom messages — has no expand affordance, so the app fetches the entry
 * itself, `get_entries{ids}`, as soon as a row that shows it renders:
 *
 * - one request per burst of rows (ids queued in a microtask), never twice for the same id
 *   (`loading` / `done`, plus a small LRU of full entries that re-applies after a reconcile
 *   re-delivers the cut copy);
 * - the full entry replaces the cut one in the store, so everything that reads entries —
 *   markdown, copy, quote, export — sees the whole text;
 * - a failure leaves `error` (with the reason) for a "Show full message" button (`retry`); a reference-only
 *   stub (no row to show a button on) is retried by itself, a few times with growing waits;
 * - each id settles alone: one failing id never fails the others of its batch, and a request that never
 *   answers ends in `error` (`FETCH_TIMEOUT_MS`) instead of `loading` for good.
 */
import { createStore } from 'solid-js/store';
import { reasonOf, type FullEntriesResult } from '../host/fullEntries';
import { isProvisional, type SessionEntry } from './types';

/** The suffix `bound.rs` appends to cut text (`TRUNCATION_NOTE`). Never shown in prose. */
export const TRUNCATION_NOTE = '\n…[truncated by the host — expand for the full output]';
const NOTE_TAIL = /\n?…\[truncated by the host — expand for the full output\]\s*$/;

/** True when the host cut `text` (its trailing truncation note is still on it). */
export function hasTruncationNote(text: string): boolean {
  return text.endsWith(']') && NOTE_TAIL.test(text);
}

/** `text` without the host's trailing truncation note (the head is kept as-is). */
export function stripTruncationNote(text: string): string {
  return text.endsWith(']') ? text.replace(NOTE_TAIL, '') : text;
}

/** A host-cut prose entry (assistant / user message, custom message) — tool and bash output expand on their own. */
export function needsFullMessage(entry: SessionEntry | undefined): boolean {
  if (!entry || isProvisional(entry.id)) return false;
  if (!entry.type && entry.truncated === true) return true; // host reference-only stub
  if (entry.type !== 'message' && entry.type !== 'custom_message') return false;
  const role = entry.message?.role;
  if (role === 'toolResult' || role === 'bashExecution') return false;
  return entry.message?.truncated === true || entry.truncated === true;
}

export type FullStatus = 'loading' | 'error';

/** A reference-only host stub: it has no row type, so nothing on screen can ask for it. */
export const isStub = (entry: SessionEntry | undefined): boolean => !!entry && !entry.type && entry.truncated === true;

/** A request that has not answered by then counts as failed (retryable). */
export const FETCH_TIMEOUT_MS = 60_000;
/** Waits before a stub's automatic retries (ms). */
export const STUB_RETRY_MS: readonly number[] = [1_500, 4_000, 10_000];

export type FullMessages = {
  /** Fetches (batched) the full entries of the cut prose among `ids`; no-op for the rest, in flight, done or failed (use `retry`). */
  ensure(ids: readonly string[]): void;
  /** Re-asks for entries whose fetch failed. */
  retry(ids: readonly string[]): void;
  /** Reactive: `loading` while asked, `error` after a failure, undefined otherwise. */
  status(id: string): FullStatus | undefined;
  /** Why the last fetch of `id` failed (reactive; only while `status(id) === 'error'`). */
  error(id: string): string | undefined;
  /** Stops the pending automatic retries (the session closed). */
  dispose(): void;
};

const CACHE_MAX = 64;

export function createFullMessages(opts: {
  /** The session's entry by id (reactive store read). */
  entry: (id: string) => SessionEntry | undefined;
  /** The whole entries; the settled form names the ids that failed (the plain list: a missing id failed). */
  getEntries: (ids: readonly string[]) => Promise<SessionEntry[] | FullEntriesResult>;
  /** Replaces the cut entries by the full ones in the store. */
  apply: (full: readonly SessionEntry[]) => void;
}): FullMessages {
  const [statuses, setStatuses] = createStore<Record<string, FullStatus | undefined>>({});
  const [errors, setErrors] = createStore<Record<string, string | undefined>>({});
  const full = new Map<string, SessionEntry>();
  const done = new Set<string>();
  const attempts = new Map<string, number>();
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let queue = new Set<string>();
  let scheduled = false;
  let disposed = false;

  const remember = (entry: SessionEntry) => {
    full.delete(entry.id);
    full.set(entry.id, entry);
    while (full.size > CACHE_MAX) {
      const oldest = full.keys().next();
      if (oldest.done) break;
      full.delete(oldest.value);
    }
  };

  const fail = (id: string, reason: string) => {
    setErrors(id, reason);
    setStatuses(id, 'error');
    // A stub has no "Show full message" button to retry from: it asks again by itself.
    const n = attempts.get(id) ?? 0;
    if (disposed || n >= STUB_RETRY_MS.length || !isStub(opts.entry(id))) return;
    attempts.set(id, n + 1);
    const timer = setTimeout(() => {
      timers.delete(timer);
      if (!disposed) ensure([id], true);
    }, STUB_RETRY_MS[n]);
    timers.add(timer);
  };

  const flush = () => {
    scheduled = false;
    const ids = [...queue];
    queue = new Set();
    if (!ids.length) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('The host did not answer in time')), FETCH_TIMEOUT_MS);
    });
    Promise.race([opts.getEntries(ids), timeout])
      .then((reply) => {
        const entries = Array.isArray(reply) ? reply : reply.entries;
        const failed = Array.isArray(reply) ? {} : reply.errors;
        // A reply that is itself a bare stub carries nothing more than what is on screen.
        const got = entries.filter((e) => ids.includes(e.id) && !isStub(e));
        for (const e of got) {
          remember(e);
          done.add(e.id);
        }
        opts.apply(got);
        for (const id of ids) {
          if (got.some((e) => e.id === id)) {
            attempts.delete(id);
            setErrors(id, undefined);
            setStatuses(id, undefined);
          } else fail(id, failed[id] ?? 'The host did not return this message');
        }
      })
      .catch((e: unknown) => {
        for (const id of ids) fail(id, reasonOf(e));
      })
      .finally(() => clearTimeout(timer));
  };

  function ensure(ids: readonly string[], retrying = false) {
    const hit: SessionEntry[] = [];
    for (const id of ids) {
      if (!needsFullMessage(opts.entry(id))) continue;
      const cached = full.get(id);
      if (cached) {
        // A remainder the frame could not carry (still flagged) is already as full as it gets.
        if (!needsFullMessage(cached)) hit.push(cached);
        continue;
      }
      if (done.has(id)) continue; // fetched once: a frame-sized remainder stays cut, never re-asked
      const status = statuses[id];
      if (status === 'loading' || (status === 'error' && !retrying)) continue;
      setStatuses(id, 'loading');
      queue.add(id);
    }
    if (hit.length) opts.apply(hit);
    if (queue.size && !scheduled) {
      scheduled = true;
      queueMicrotask(flush);
    }
  }

  return {
    ensure: (ids) => ensure(ids),
    retry: (ids) => {
      for (const id of ids) attempts.delete(id);
      ensure(ids, true);
    },
    status: (id) => statuses[id],
    error: (id) => (statuses[id] === 'error' ? errors[id] : undefined),
    dispose: () => {
      disposed = true;
      for (const t of timers) clearTimeout(t);
      timers.clear();
    },
  };
}
