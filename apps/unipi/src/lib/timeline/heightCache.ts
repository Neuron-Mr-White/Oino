/**
 * Row-height cache (app spec §3.6, WP-A6; port of ZCode
 * `v4/timelineRowHeightCache.ts`): keyed by the stable `turnId`, it survives
 * virtualizer resets (entries rebuild, session reopen, live-tail migration),
 * so remounting a row never falls back to a blind estimate and the bottom
 * anchor doesn't jump.
 *
 * The virtualizer's own measurementsCache already keys by item key, but a
 * count/array rebuild clears it; this cache is the component-instance
 * persistent layer. Cleared on session switch (turnIds are not unique
 * across sessions).
 */

/** Estimate for a turn that was never measured (ZCode ROW_ESTIMATE_PX). */
export const DEFAULT_TURN_HEIGHT = 72;

/** Max entries; the least recently *written* turn is evicted first. */
export const HEIGHT_CACHE_MAX = 4000;

export class TurnHeightCache {
  private readonly sizes = new Map<string, number>();

  constructor(private readonly max = HEIGHT_CACHE_MAX) {}

  get size(): number {
    return this.sizes.size;
  }

  /** A real measurement. Rewriting refreshes the eviction order (an active turn stays warm). */
  set(turnId: string, heightPx: number): void {
    if (!Number.isFinite(heightPx) || heightPx <= 0) return;
    this.sizes.delete(turnId);
    this.sizes.set(turnId, heightPx);
    while (this.sizes.size > this.max) {
      const oldest = this.sizes.keys().next();
      if (oldest.done) break;
      this.sizes.delete(oldest.value);
    }
  }

  get(turnId: string): number | undefined {
    return this.sizes.get(turnId);
  }

  /** `estimateSize` entry point: measured height, else the fallback estimate. */
  estimate(turnId: string | undefined, fallback = DEFAULT_TURN_HEIGHT): number {
    if (turnId === undefined) return fallback;
    return this.sizes.get(turnId) ?? fallback;
  }

  /** Session switch: turnIds are per-session. */
  clear(): void {
    this.sizes.clear();
  }
}
