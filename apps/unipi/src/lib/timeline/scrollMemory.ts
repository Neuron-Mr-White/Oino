/**
 * Per-timeline scroll memory (app spec §3.3/§3.6, WP-A6; ZCode
 * `chatSessionScrollMemory.ts`): keyed `(paneId, hostId, sessionId)`, an
 * in-memory LRU of at most 200 entries (renderer-local only — never
 * localStorage, the relay or the host).
 *
 * Leaving a conversation stores either the bottom-lock flag or a pixel
 * `scrollTop`; coming back restores in layout, re-corrects once after the
 * first measure frame, and yields to any real user scroll.
 */

/** Max remembered positions (ZCode's cap). */
export const SCROLL_MEMORY_MAX = 200;

/** [chat-load] `anchor`: the first visible turn and how far the viewport top was below its top. */
export type ScrollMemoryEntry = { bottomLocked: true } | { scrollTop: number; anchor?: { turnId: string; offset: number } };

const memory = new Map<string, ScrollMemoryEntry>();

export function scrollMemoryKey(paneId: string, hostId: string, sessionId: string): string {
  return `${paneId}|${hostId}|${sessionId}`;
}

/** What a timeline saw last; `undefined` = open pinned to the bottom. */
export function readScrollMemory(key: string): ScrollMemoryEntry | undefined {
  return memory.get(key);
}

/** Stores the position; refreshes the LRU order and evicts the coldest entry past 200. */
export function writeScrollMemory(key: string, entry: ScrollMemoryEntry): void {
  memory.delete(key);
  memory.set(key, entry);
  while (memory.size > SCROLL_MEMORY_MAX) {
    const oldest = memory.keys().next();
    if (oldest.done) break;
    memory.delete(oldest.value);
  }
}

export function scrollMemorySize(): number {
  return memory.size;
}

/** Tests. */
export function clearScrollMemory(): void {
  memory.clear();
}
