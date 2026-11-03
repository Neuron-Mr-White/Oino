/**
 * Extension render cache (app spec §3.4 step 7, §3.2; WP-A8):
 * `_oino/render` answers for one transcript row (ui-protocol §6.4,
 * §9.3) — `customMessage` / `customEntry` cards, `tool` render-only
 * overrides, `turnFooter` (A27). Cached by `(target, key, entryId, rev,
 * width)` so a virtual-scroll remount never re-asks; in-flight requests
 * dedupe; a 2 s timeout or error (the stream applies it) resolves `null` —
 * **nothing renders** and the frontend's own row stays (core's `null` =
 * "the frontend's own rendering", never an error).
 */
import { createSignal } from 'solid-js';
import type { SessionHandle } from './store';
import type { UiNode } from '../../ui/types';

export type RenderTargetName = 'customMessage' | 'customEntry' | 'tool' | 'turnFooter';
export type RenderDensity = 'simple' | 'regular' | 'advanced';

/** ui-protocol §6.1 width classes (60/100 terminal columns ≈ px at ~9 px/col). */
export type RenderWidthClass = 'narrow' | 'normal' | 'wide';
export function widthClassOf(px: number): RenderWidthClass {
  if (px < 560) return 'narrow';
  if (px >= 900) return 'wide';
  return 'normal';
}

/** The cache key of a row (app spec §3.4: `(target, key, entryId, rev, width)`). */
export type RenderRowKey = {
  target: RenderTargetName;
  /** Custom type or tool name. */
  key: string;
  entryId?: string;
  /** Content revision (a UiEntry `rev`, A27's invalidate counter); immutable rows pass 0. */
  rev?: number;
  width: RenderWidthClass;
};

/** Everything but the cached key parts — what the request carries. */
export type RenderRowInput = {
  density?: RenderDensity;
  expanded?: boolean;
  running?: boolean;
  turnEntryId?: string;
  turnIndex?: number;
  /** Row payload: the entry/message JSON, or `{toolCall, args, result?, partialResult?, isError?}`. */
  data: unknown;
};

const keyOf = (r: RenderRowKey): string => `${r.target}|${r.key}|${r.entryId ?? ''}|${r.rev ?? 0}|${r.width}`;

export const RENDER_CACHE_MAX = 500;
/** Live per-session caches (twice the session store LRU: a few slack for handovers; perf-app). */
const RENDER_SESSIONS_MAX = 6;
const perSession = new Map<string, RenderCache>();

/** The session's render cache (one per open session; `session.key` scoped). */
export function sessionRenderCache(session: SessionHandle): RenderCache {
  let cache = perSession.get(session.key);
  if (!cache) {
    cache = createRenderCache(session);
    perSession.delete(session.key);
    perSession.set(session.key, cache);
    while (perSession.size > RENDER_SESSIONS_MAX) {
      const oldest = perSession.keys().next();
      if (oldest.done) break;
      perSession.delete(oldest.value);
    }
  }
  return cache;
}

/** Test seam: drop the per-session caches. */
export function resetSessionRenderCaches(): void {
  perSession.clear();
}

type Slot = { read: () => UiNode | null | undefined };

export type RenderCache = ReturnType<typeof createRenderCache>;

/**
 * One cache per session (`createRenderCache(session)`): renders go out on
 * its stream (`cmd`), so `E_TIMEOUT`/closed-stream rejections settle to
 * `null` without a retry storm. Dispose on session close.
 */
export function createRenderCache(session: SessionHandle) {
  const slots = new Map<string, Slot>();

  const fetch = (row: RenderRowKey, input: RenderRowInput): Slot => {
    const [read, write] = createSignal<UiNode | null | undefined>(undefined);
    const cmd: Record<string, unknown> = {
      type: '_oino/render',
      target: row.target,
      key: row.key,
      ...(row.entryId !== undefined ? { entryId: row.entryId } : {}),
      ...(input.turnEntryId !== undefined ? { turnEntryId: input.turnEntryId } : {}),
      ...(input.turnIndex !== undefined ? { turnIndex: input.turnIndex } : {}),
      ...(input.running !== undefined ? { running: input.running } : {}),
      density: input.density ?? 'regular',
      expanded: input.expanded === true,
      width: row.width,
      data: input.data,
    };
    session
      .cmd<{ node?: unknown }>(cmd as never)
      .then((reply) => write(((reply as { node?: unknown } | null)?.node as UiNode | undefined) ?? null), () => write(null));
    return { read };
  };

  return {
    /**
     * The rendered node for `row` (`undefined` while in flight → render
     * nothing yet; `null` = none / timed out → the frontend's own row).
     */
    node(row: RenderRowKey, input: RenderRowInput): () => UiNode | null | undefined {
      const k = keyOf(row);
      let slot = slots.get(k);
      if (!slot) {
        slot = fetch(row, input);
        slots.delete(k);
        slots.set(k, slot);
        while (slots.size > RENDER_CACHE_MAX) {
          const oldest = slots.keys().next();
          if (oldest.done) break;
          slots.delete(oldest.value);
        }
      }
      return slot.read;
    },
    /** Drops cached rows (`invalidateTurnFooter` bumps `rev` instead; this is for tests/scope ends). */
    drop(match: (row: RenderRowKey) => boolean): void {
      for (const k of [...slots.keys()]) {
        const [target, key, entryId, rev, width] = k.split('|');
        if (match({ target: target as RenderTargetName, key, entryId: entryId || undefined, rev: Number(rev) || 0, width: width as RenderWidthClass })) slots.delete(k);
      }
    },
    /** Test seam: how many rows are cached. */
    get size(): number {
      return slots.size;
    },
  };
}
