/**
 * [chat-load] The timeline's row model (design: oino-internal
 * docs/dev/v2/design/chat-render-pipeline.md §New.e): heights per turn, cached PER SESSION
 * across opens, stable estimates per turn kind, prefix offsets and the window.
 *
 * - A measured height is the row's real border-box height (rows are in normal flow, so the
 *   DOM and this model agree once a row was measured).
 * - An unmeasured turn uses the running mean of the measured turns of its KIND in this
 *   session (fold / plain / compaction / preamble), else a fixed default per kind: the
 *   estimate of a turn never changes because a neighbour was measured, only by its kind's
 *   mean, and it is replaced by the real height the first time the row mounts.
 * - Memory is bounded: `SESSIONS_MAX` sessions (LRU) × `HEIGHT_CACHE_MAX` turns each.
 */
import { HEIGHT_CACHE_MAX, TurnHeightCache } from './heightCache';
import type { Turn } from '../session/turns';

export type RowKind = 'fold' | 'plain' | 'compaction' | 'preamble';

/** Defaults before any turn of the kind was measured (desktop column, collapsed folds). */
export const KIND_DEFAULT: Record<RowKind, number> = { fold: 220, plain: 140, compaction: 48, preamble: 64 };

export function rowKind(t: Turn): RowKind {
  if (t.kind === 'compaction') return 'compaction';
  if (t.kind === 'preamble') return 'preamble';
  return t.segments.some((s) => s.fold) ? 'fold' : 'plain';
}

/** One session's heights. */
export class SessionHeights {
  readonly measured: TurnHeightCache;
  /** The estimate a turn got the first time the layout used it (kept until it is measured). */
  private readonly assumed: TurnHeightCache;
  private readonly means = new Map<RowKind, { sum: number; n: number }>();

  constructor(max = HEIGHT_CACHE_MAX) {
    this.measured = new TurnHeightCache(max);
    this.assumed = new TurnHeightCache(max);
  }

  /** A real measurement of `t`'s row. Returns whether it changed (> 0.5 px). */
  set(t: Pick<Turn, 'turnId'> & Partial<Turn>, px: number): boolean {
    if (!Number.isFinite(px) || px <= 0) return false;
    const old = this.measured.get(t.turnId);
    if (old !== undefined && Math.abs(old - px) <= 0.5) return false;
    this.measured.set(t.turnId, px);
    if (old === undefined && t.segments) {
      const k = rowKind(t as Turn);
      const m = this.means.get(k) ?? { sum: 0, n: 0 };
      m.sum += px;
      m.n += 1;
      this.means.set(k, m);
    }
    return true;
  }

  get(turnId: string): number | undefined {
    return this.measured.get(turnId);
  }

  /**
   * The height the layout uses for `t`: measured, else the estimate it got the first time
   * (its kind's mean then, else the default) — an unmeasured turn's estimate never moves,
   * so nothing above the reader shifts because some other row was measured.
   */
  height(t: Turn): number {
    const m = this.measured.get(t.turnId);
    if (m !== undefined) return m;
    const a = this.assumed.get(t.turnId);
    if (a !== undefined) return a;
    const k = rowKind(t);
    const mean = this.means.get(k);
    const est = mean && mean.n > 0 ? Math.round(mean.sum / mean.n) : KIND_DEFAULT[k];
    this.assumed.set(t.turnId, est);
    return est;
  }
}

/** Sessions whose heights are remembered (reopening a chat lays out at its real heights). */
export const SESSIONS_MAX = 8;
const sessions = new Map<string, SessionHeights>();

export function sessionHeights(key: string): SessionHeights {
  let h = sessions.get(key);
  if (h) sessions.delete(key);
  else h = new SessionHeights();
  sessions.set(key, h);
  while (sessions.size > SESSIONS_MAX) {
    const oldest = sessions.keys().next();
    if (oldest.done) break;
    sessions.delete(oldest.value);
  }
  return h;
}

/** Tests. */
export function clearSessionHeights(): void {
  sessions.clear();
}

/** Prefix offsets: `offsets[i]` = top of turn `i` in the list, `offsets[n]` = list height. */
export function prefixOffsets(turns: readonly Turn[], heights: SessionHeights): Float64Array {
  const out = new Float64Array(turns.length + 1);
  for (let i = 0; i < turns.length; i++) out[i + 1] = out[i]! + heights.height(turns[i]!);
  return out;
}

/** First index whose row ends below `y` (binary search; `n` when none). */
export function indexAt(offsets: Float64Array, y: number): number {
  const n = offsets.length - 1;
  let lo = 0;
  let hi = n;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid + 1]! > y) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/** The rows `[start, end)` that intersect `[top, bottom]` of the list. */
export function windowOf(offsets: Float64Array, top: number, bottom: number): [number, number] {
  const n = offsets.length - 1;
  if (n <= 0) return [0, 0];
  const start = Math.min(n - 1, indexAt(offsets, Math.max(0, top)));
  let end = start;
  while (end < n && offsets[end]! < bottom) end++;
  return [start, Math.max(start + 1, end)];
}
