/**
 * Expansion memory (app spec §3.3 `expansion`, §3.6 lazy detail, WP-A6):
 * whether a fold, tool body, thinking block or group is open, keyed
 * `(sessionId, key)`. Memory-only, LRU 5000 — virtual-scroll unmounts never
 * lose a user's expansion, so a folded segment stays folded (or open) when
 * it leaves and re-enters the viewport.
 *
 * Reads are reactive (a versioned signal), so toggling re-renders the row.
 */
import { createSignal } from 'solid-js';

/** Max remembered expansions. */
export const EXPANSION_MAX = 5000;

const entries = new Map<string, boolean>();
const [rev, setRev] = createSignal(0);

const keyOf = (sessionId: string, key: string) => `${sessionId}|${key}`;

/**
 * The open state of `key` in `sessionId`; `fallback` when the user never
 * toggled it (folds: open only while forced or asked; tools/thinking:
 * closed).
 */
export function expansionOpen(sessionId: string, key: string, fallback: boolean): boolean {
  rev();
  return entries.get(keyOf(sessionId, key)) ?? fallback;
}

/** Writes the state; refreshes the LRU order. */
export function setExpansion(sessionId: string, key: string, open: boolean): void {
  const k = keyOf(sessionId, key);
  entries.delete(k);
  entries.set(k, open);
  while (entries.size > EXPANSION_MAX) {
    const oldest = entries.keys().next();
    if (oldest.done) break;
    entries.delete(oldest.value);
  }
  setRev((n) => n + 1);
}

/** Toggles, honouring `fallback` for a first touch. */
export function toggleExpansion(sessionId: string, key: string, fallback: boolean): boolean {
  const next = !(entries.get(keyOf(sessionId, key)) ?? fallback);
  setExpansion(sessionId, key, next);
  return next;
}

/** Drops a session's entries (session closed for good). */
export function dropSessionExpansion(sessionId: string): void {
  let dirty = false;
  for (const k of entries.keys()) {
    if (k.startsWith(`${sessionId}|`)) {
      entries.delete(k);
      dirty = true;
    }
  }
  if (dirty) setRev((n) => n + 1);
}

/** Tests. */
export function clearExpansion(): void {
  entries.clear();
  setRev((n) => n + 1);
}

export function expansionSize(): number {
  rev();
  return entries.size;
}
