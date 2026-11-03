/**
 * Static markdown cache (app spec §3.6 Z-34, WP-A6): a completed prose part
 * renders exactly once — the `renderBlocks` output (sanitised HTML strings)
 * is cached by `entryId | width class`, so virtual-scroll unmount/remount
 * and fold open/close never re-run the markdown pipeline. Only the truly
 * streaming part takes the incremental path, and nothing fades in.
 *
 * Width class quantizes the column (64 px): a rewrap-producing width change
 * re-renders at a coarser grain instead of one entry per pixel.
 */
import { renderBlocks } from '../chat/markdown';

/** Max cached renders (entry × width class). */
export const MD_CACHE_MAX = 600;
/** Pixels per width class. */
const WIDTH_CLASS_PX = 64;

/** perf-app: total cached HTML (UTF-16 chars) — a few huge code blocks cannot pin 600 × megabytes. */
export const MD_CACHE_CHARS_MAX = 4_000_000;
/** A single render larger than this is never cached (it re-renders on remount). */
const MD_ENTRY_CHARS_MAX = 400_000;

const cache = new Map<string, string[]>();
const sizes = new Map<string, number>();
let chars = 0;
const sizeOf = (blocks: string[]): number => blocks.reduce((n, b) => n + b.length, 0);

export const widthClassOf = (px: number): number => Math.max(0, Math.floor(px / WIDTH_CLASS_PX));

/**
 * The blocks of a *settled* `text` keyed `key` (entry id + part index),
 * rendered at width `widthPx`. Live text bypasses the cache (callers pass
 * `live` parts straight to `renderBlocks`).
 */
export function mdBlocks(key: string, widthPx: number, text: string): string[] {
  const k = `${key}|${widthClassOf(widthPx)}`;
  const hit = cache.get(k);
  if (hit) {
    // Refresh recency (read-mostly LRU).
    cache.delete(k);
    cache.set(k, hit);
    return hit;
  }
  const blocks = renderBlocks(text, false);
  const size = sizeOf(blocks);
  if (size > MD_ENTRY_CHARS_MAX) return blocks;
  cache.set(k, blocks);
  sizes.set(k, size);
  chars += size;
  while (cache.size > MD_CACHE_MAX || chars > MD_CACHE_CHARS_MAX) {
    const oldest = cache.keys().next();
    if (oldest.done) break;
    cache.delete(oldest.value);
    chars -= sizes.get(oldest.value) ?? 0;
    sizes.delete(oldest.value);
  }
  return blocks;
}

/** Tests / width sweeps. */
export function clearMdCache(): void {
  cache.clear();
  sizes.clear();
  chars = 0;
}

export function mdCacheSize(): number {
  return cache.size;
}
