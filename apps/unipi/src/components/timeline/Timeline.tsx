/**
 * The conversation timeline (WP-A6, §3.6 D-8): a `@tanstack/solid-virtual`
 * list over **settled turns** (item key = `turnId`) + the live tail outside
 * it, a single bottom-lock follow state, scroll memory, LRU height cache,
 * and the sticky bottom dock (Z-66) the later WPs fill.
 *
 * Scroll rules (ZCode `chat-scroll-bottom-pinning.md`):
 * - content grows while locked → pin to the bottom in the same frame;
 * - user intent (wheel/touch/key/scrollbar) + > 48 px off the bottom → detach;
 * - back within 48 px or the jump button → re-lock; new arrivals while
 *   detached set the unread dot (Z-24);
 * - width changes while locked → measure-then-pin synchronously;
 * - the live tail migrates into the list with its height pre-seeded.
 *
 * `Timeline` is keyed per session by `Conversation`; scroll memory +
 * height cache live and die with it.
 */
import { createEffect, createMemo, createSignal, Index, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { createVirtualizer } from '@tanstack/solid-virtual';
import { measureElement as defaultMeasure, observeElementOffset, observeElementRect, type Virtualizer } from '@tanstack/virtual-core';
import { RefreshCw, TriangleAlert } from 'lucide-solid';
import { createBottomLock, distanceToBottom } from '../../lib/timeline/bottomLock';
import { TurnHeightCache, DEFAULT_TURN_HEIGHT } from '../../lib/timeline/heightCache';
import { readScrollMemory, writeScrollMemory, scrollMemoryKey } from '../../lib/timeline/scrollMemory';
import { readTimelineSnapshot, writeTimelineSnapshot } from '../../lib/timeline/snapshotCache';
import { EmptyState } from '../EmptyState';
import { LiveTail } from './LiveTail';
import { JumpButton } from './JumpButton';
import { ChatSkeleton } from './Skeleton';
import { TurnRow } from './TurnRow';
import { TurnNavigator } from './TurnNavigator';
import { SelectionQuote } from './SelectionQuote';
import { TurnMenus } from './TurnActions';
import { actionNotice } from '../../lib/timeline/actions';
import { FindBar } from '../command/FindBar';
import { activateFindMark, applyFindHighlights, clearFindHighlights } from '../../lib/find/highlight';
import { buildFindIndex, closeFind, findState, locateMatch, setFindAdapter, type FindIndex } from '../../lib/find';
import type { Turn } from '../../lib/session/turns';
import type { TimelineCtx } from './context';

/** Perf marks land here for the capture harness (§3.6 budgets). */
declare global {
  interface Window {
    __UNIPI_PERF__?: Array<{ name: string; ms: number; detail?: unknown }>;
  }
}

function perf(name: string, start: number): void {
  const ms = performance.now() - start;
  try {
    performance.measure(name, { start });
  } catch {
    // marks are best-effort
  }
  (window.__UNIPI_PERF__ ??= []).push({ name, ms });
}

export function Timeline(props: { ctx: TimelineCtx; dock?: JSX.Element; /** U10: the `header` slot band (scrolls with the transcript). */ header?: JSX.Element; /** U10: the `overlay` slot host (below the dock, §6.4). */ overlay?: JSX.Element; /** Z-87: the new-chat empty state (supplied by `Conversation`, which knows the host). */ empty?: JSX.Element }) {
  const session = () => props.ctx.session;
  const liveTurns = () => session().turns();
  const cachedTurns = createMemo(() => readTimelineSnapshot(session().key));
  const turns = createMemo(() => {
    const live = liveTurns();
    if (live.length > 0 || session().state.conn === 'live') return live;
    return cachedTurns() ?? live;
  });
  createEffect(() => {
    const live = liveTurns();
    if (live.length > 0) writeTimelineSnapshot(session().key, live);
  });
  /** The running turn (outside the list); everything before it is virtualized. */
  const liveTurn = () => {
    const t = turns();
    const last = t[t.length - 1];
    return last?.live ? last : undefined;
  };
  const settled = () => {
    const t = turns();
    return t.length && t[t.length - 1]!.live ? t.slice(0, -1) : t;
  };

  const heights = new TurnHeightCache();
  const lock = createBottomLock();
  const [scrollEl, setScrollEl] = createSignal<HTMLDivElement>();
  const [overflowing, setOverflowing] = createSignal(false);
  const t0 = performance.now();

  // The virtualizer lives in `VirtualTurns`, created only once the scroller
  // element exists (its `_willUpdate` binds the scroll/rect observers —
  // constructing it before the ref lands leaves them on a null element).
  const [virt, setVirt] = createSignal<Virtualizer<HTMLDivElement, HTMLDivElement> | undefined>();

  // ── scroll container wiring ──────────────────────────────────────
  let contentEl: HTMLDivElement | undefined;
  let accordionToggleUntil = 0;
  const syncOverflow = () => {
    const el = scrollEl();
    if (el) setOverflowing(el.scrollHeight - el.clientHeight > 4);
  };

  onMount(() => {
    const el = scrollEl();
    if (!el) return;
    const intent = () => lock.intent();
    const onPointerDown = (e: PointerEvent) => {
      // Scrollbar track/thumb press on the scroll container itself counts as user scroll intent;
      // clicking buttons/accordions inside `.tl-content` or `.tl-dock` must not fake a scroll.
      if (e.target === el) lock.intent();
    };
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.closest?.('input, textarea, [contenteditable="true"]')) return;
      // Keys that scroll the container count as user intent.
      if (['PageUp', 'PageDown', 'ArrowUp', 'ArrowDown', 'Home', 'End', ' '].includes(e.key)) intent();
    };
    el.addEventListener('wheel', intent, { passive: true });
    el.addEventListener('touchstart', intent, { passive: true });
    el.addEventListener('pointerdown', onPointerDown, { passive: true });
    el.addEventListener('keydown', onKey);
    onCleanup(() => {
      el.removeEventListener('wheel', intent);
      el.removeEventListener('touchstart', intent);
      el.removeEventListener('pointerdown', onPointerDown);
      el.removeEventListener('keydown', onKey);
    });

    // Store-driven growth (entries, live tail, outbox, new turns): same-
    // frame signal — pin while locked, unread dot while detached. RO below
    // covers intrinsic resizes this can't see (image decode, fold toggle).
    createEffect(() => {
      session().state.entriesRev;
      liveTurn();
      session().state.outbox.length;
      settled().length;
      if (!restore.done) return;
      lock.grew(el);
      syncOverflow();
    });

    if (typeof ResizeObserver !== 'undefined') {
      // Content growth (stream, folds, measure, dock): pin while locked,
      // unread dot while detached. User-initiated accordion toggles in a
      // settled turn re-measure in place without yanking scrollTop or
      // lighting up the unread dot.
      const grow = new ResizeObserver(() => {
        if (!restore.done) return;
        if (!liveTurn() && performance.now() < accordionToggleUntil) {
          virt()?.measure();
          lock.reconcile(el);
          syncOverflow();
          return;
        }
        lock.grew(el);
        syncOverflow();
      });
      if (contentEl) grow.observe(contentEl);
      // Viewport changes: synchronous measure-then-pin while locked (the
      // ZCode width-transition batch); re-check overflow either way.
      let lastW = el.clientWidth;
      const view = new ResizeObserver(() => {
        if (el.clientWidth !== lastW) {
          lastW = el.clientWidth;
          virt()?.measure();
        }
        if (lock.locked()) lock.pin(el);
        lock.reconcile(el);
        syncOverflow();
      });
      view.observe(el);
      onCleanup(() => {
        grow.disconnect();
        view.disconnect();
      });
    }
  });

  // ── live tail → list migration: pre-seed its measured height ─────
  let tailEl: HTMLDivElement | undefined;
  createEffect(
    on(liveTurn, (cur, prev) => {
      if (prev && !cur) heights.set(prev.turnId, tailEl?.offsetHeight || heights.estimate(prev.turnId));
    }),
  );

  // ── anchor preservation on branch/history changes (entriesRev) ────
  createEffect(
    on(
      () => session().state.entriesRev,
      () => {
        const el = scrollEl();
        if (!el || !restore.done || lock.locked()) return;
        const items = virt()?.getVirtualItems() ?? [];
        const first = items.find((i) => i.end > el.scrollTop) ?? items[0];
        const anchor = first ? { key: String(first.key), delta: el.scrollTop - first.start } : undefined;
        queueMicrotask(() => {
          if (!anchor || lock.locked()) return;
          const idx = settled().findIndex((t) => t.turnId === anchor.key);
          if (idx < 0) return;
          virt()?.scrollToIndex(idx, { align: 'start' });
          el.scrollTop += anchor.delta;
        });
      },
    ),
  );

  // ── scroll memory: restore once content exists ────────────────────
  const memKey = () => scrollMemoryKey('main', props.ctx.hostId, props.ctx.sessionId());
  const saved = readScrollMemory(memKey());
  // onCleanup must not read props.ctx: the parent's memos are already disposed
  // when this component is cleaned up (Solid cleanNode crash on route change).
  let lastMemKey = memKey();
  createEffect(() => {
    lastMemKey = memKey();
  });
  const restore: { pos: 'bottom' | number; done: boolean } = { pos: saved && 'scrollTop' in saved ? saved.scrollTop : 'bottom', done: false };
  let userScrolled = false;
  const origIntent = lock.intent;
  lock.intent = () => {
    userScrolled = true;
    origIntent();
  };
  if (typeof restore.pos === 'number') lock.detach();
  const applyRestore = () => {
    const el = scrollEl();
    if (!el) return;
    if (restore.pos === 'bottom') lock.jump(el);
    else {
      el.scrollTop = Math.min(restore.pos, Math.max(0, el.scrollHeight - el.clientHeight));
      lock.reconcile(el);
    }
  };
  createEffect(() => {
    const el = scrollEl();
    if (restore.done || !el || settled().length === 0) return;
    restore.done = true;
    // Layout write now, then one correction after the first measure frame —
    // which yields if the user already scrolled (ZCode rule).
    applyRestore();
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        if (!userScrolled) applyRestore();
        perf('tl:first-paint', t0);
      }),
    );
  });
  // Fallback first-paint mark for empty sessions.
  onMount(() => {
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        if (!restore.done) perf('tl:first-paint', t0);
      }),
    );
  });
  onCleanup(() => {
    const el = scrollEl();
    if (!el) return;
    writeScrollMemory(lastMemKey, lock.locked() ? { bottomLocked: true } : { scrollTop: el.scrollTop });
  });

  const loading = () => session().state.conn === 'connecting' && turns().length === 0;
  const openFailed = () => session().state.conn === 'closed' && !!session().state.connError && turns().length === 0;
  const empty = () => session().state.conn === 'live' && turns().length === 0 && !session().state.outbox.length;

  // ── navigator (Z-25): the query at the top of the viewport is active ─
  const [activeTurn, setActiveTurn] = createSignal<string>();
  const updateActive = () => {
    const el = scrollEl();
    if (!el) return;
    const first = (virt()?.getVirtualItems() ?? []).find((item) => item.end > el.scrollTop + 2);
    const turn = first ? settled()[first.index] : undefined;
    const live = liveTurn();
    setActiveTurn(turn?.turnId ?? live?.turnId);
  };
  createEffect(() => {
    settled().length;
    liveTurn();
    queueMicrotask(updateActive);
  });

  /** Navigator jump: mount the turn, land it at the top of the viewport, detach (Z-25). */
  const jumpToTurn = (turnId: string) => {
    const el = scrollEl();
    if (!el) return;
    // A positional jump is the reader's intent: the restore correction must
    // not pull the view back to the bottom in the next frame.
    userScrolled = true;
    lock.detach();
    lock.seen();
    const index = settled().findIndex((turn) => turn.turnId === turnId);
    if (index >= 0) virt()?.scrollToIndex(index, { align: 'start' });
    else if (tailEl) el.scrollTop = tailEl.offsetTop;
    updateActive();
  };

  // ── find in chat (WP-A18, Z-79) ─────────────────────────────────
  const findIdx = createMemo(() => buildFindIndex(turns(), findState().query));
  /** The hit entry id's turn (whole-session results jump through this). */
  const turnOfEntry = (entryId: string) => turns().find((t) => t.entries.some((e) => e.id === entryId))?.turnId;
  onMount(() => {
    setFindAdapter({ jumpToTurn });
    onCleanup(() => setFindAdapter(undefined));
  });
  // Rows mount/unmount under the virtualizer: re-highlight what is mounted.
  // Re-applying is idempotent (text inside `mark.find-hit` is skipped), so the
  // pass that our own marks trigger adds nothing and the loop stops there.
  const [findDomRev, setFindDomRev] = createSignal(0);
  createEffect(() => {
    const root = contentEl;
    const open = findState().open;
    if (!root || !open) return;
    const mo = new MutationObserver(() => setFindDomRev((n) => n + 1));
    mo.observe(root, { childList: true, subtree: true });
    onCleanup(() => mo.disconnect());
  });
  /** Marks `target` active; `scroll` off when only new rows mounted. */
  const showTarget = (root: HTMLElement, index: FindIndex, active: number, scroll: boolean) => {
    const target = locateMatch(index, active);
    if (!target) return;
    if (activateFindMark(root, target.turnId, target.occurrence, scroll)) return;
    // The turn is outside the virtual window: mount + centre it, then mark.
    if (!scroll) return;
    const at = settled().findIndex((t) => t.turnId === target.turnId);
    if (at < 0) return;
    const el = scrollEl();
    if (el) {
      userScrolled = true;
      lock.detach();
    }
    virt()?.scrollToIndex(at, { align: 'center' });
    const retry = () => {
      // jsdom (and smooth-scroll setups) deliver no scroll event for a
      // programmatic offset: dispatch it, then force the virtualizer to
      // recompute its window from the new offset.
      scrollEl()?.dispatchEvent(new Event('scroll'));
      virt()?.measure();
      const fresh = findIdx();
      clearFindHighlights(root);
      applyFindHighlights(root, fresh.query);
      const again = locateMatch(fresh, findState().active);
      if (again) activateFindMark(root, again.turnId, again.occurrence, true);
    };
    queueMicrotask(retry);
    requestAnimationFrame(() => queueMicrotask(retry));
  };
  // The query/open/active changed: redo the highlights from scratch.
  createEffect(() => {
    const root = contentEl;
    const s = findState();
    const index = findIdx();
    if (!root) return;
    clearFindHighlights(root);
    if (!s.open || !index.query) return;
    applyFindHighlights(root, index.query);
    showTarget(root, index, s.active, true);
  });
  // New rows mounted under the marks (virtualizer): highlight just those.
  createEffect(() => {
    findDomRev();
    const root = contentEl;
    const s = findState();
    const index = findIdx();
    if (!root || !s.open || !index.query) return;
    applyFindHighlights(root, index.query);
    showTarget(root, index, s.active, false);
  });
  // Leaving the conversation closes find (one surface, scoped to the chat).
  onCleanup(() => closeFind());

  return (
    <div class="tl-outer">
      <FindBar session={props.ctx.session} index={findIdx} turnOf={turnOfEntry} jump={jumpToTurn} />
      <TurnNavigator turns={turns} width={props.ctx.columnWidth} activeId={activeTurn} onJump={jumpToTurn} />
      <SelectionQuote root={scrollEl} sessionKey={() => props.ctx.session.key} />
      <TurnMenus ctx={props.ctx} />
      <div
        class="tl"
        ref={setScrollEl}
        data-testid="tl"
        onScroll={() => {
          const el = scrollEl();
          if (el) {
            lock.onScroll(el);
            syncOverflow();
            updateActive();
          }
        }}
      >
      <div
        class="tl-content"
        ref={contentEl}
        onClick={(e) => {
          if ((e.target as HTMLElement | null)?.closest?.('[aria-expanded], [data-testid="tl-bubble-fold"], [data-testid="turn-files-toggle"]')) {
            accordionToggleUntil = performance.now() + 350;
          }
        }}
      >
        {props.header}
        <Show when={loading()}>
          <ChatSkeleton />
        </Show>
        <Show when={openFailed()}>
          <div class="tl-empty" data-testid="tl-open-failed">
            <EmptyState
              compact
              tone="error"
              icon={<TriangleAlert size={20} />}
              title="Couldn't open this session"
              body={session().state.connError?.message || 'The host could not load this conversation.'}
              testId="tl-open-failed-card"
              action={
                <button type="button" class="btn btn-secondary btn-sm" onClick={() => session().retry()}>
                  <RefreshCw size={14} /> Retry
                </button>
              }
            />
          </div>
        </Show>
        <Show when={empty()}>
          <div class="tl-empty" data-testid="tl-empty">
            {props.empty ?? <p>Say hi — the first message starts the session.</p>}
          </div>
        </Show>
        <Show when={scrollEl()} keyed>
          {(el) => <VirtualTurns scroller={el} ctx={props.ctx} settled={settled} live={() => !!liveTurn()} heights={heights} onVirt={setVirt} />}
        </Show>
        <div ref={tailEl} class="tl-tail-wrap">
          <LiveTail live={liveTurn()} ctx={props.ctx} />
        </div>
      </div>
      <div class="tl-dock" data-testid="tl-dock">
        <Show when={actionNotice()}>
          <div class="tl-notice" role="status" data-testid="tl-notice">
            {actionNotice()}
          </div>
        </Show>
        <JumpButton show={lock.detached() && overflowing()} unread={lock.unread()} onJump={() => scrollEl() && lock.jump(scrollEl()!)} />
        {props.dock}
        <div class="tl-overlay-host" data-testid="tl-overlay-host">
          {props.overlay}
        </div>
      </div>
      </div>
    </div>
  );
}

/**
 * Measures one row under its *current* key, always. `measureElement` alone
 * is not enough when a row's key changes while the list scrolls (the bottom
 * lock pins on every growth): the virtualizer skips measuring while
 * `isScrolling`, and the ResizeObserver already watching the node does not
 * report again when its size did not change — the new key kept its estimate
 * (72 px for a settled `/graph` turn of 334 px) and the next turn and the
 * live tail drew over it. `resizeItem` writes the size regardless.
 */
export function measureRow(
  virt: Pick<Virtualizer<HTMLDivElement, HTMLDivElement>, 'measureElement' | 'resizeItem'>,
  node: HTMLElement,
): void {
  if (!node.isConnected) return;
  virt.measureElement(node as HTMLDivElement);
  const index = Number(node.dataset.index);
  // 0 = not laid out (hidden pane, jsdom): leave the estimate.
  if (Number.isInteger(index) && index >= 0 && node.offsetHeight > 0) virt.resizeItem(index, node.offsetHeight);
}

/**
 * The virtualized settled turns. Only mounts once the `.tl` scroller
 * exists, so `createVirtualizer`'s mount binds real scroll/rect observers
 * (the element is a prop, not a signal read after the fact).
 */
function VirtualTurns(props: {
  scroller: HTMLDivElement;
  ctx: TimelineCtx;
  settled: () => Turn[];
  live: () => boolean;
  heights: TurnHeightCache;
  onVirt: (v: Virtualizer<HTMLDivElement, HTMLDivElement> | undefined) => void;
}) {
  const virt = createVirtualizer<HTMLDivElement, HTMLDivElement>({
    get count() {
      return props.settled().length;
    },
    getScrollElement: () => props.scroller,
    // `_willUpdate` derives `targetWindow` from `ownerDocument.defaultView`;
    // in envs where that's null (some test jsdoms) the stock observers
    // silently bail — give them a usable window instead.
    observeElementRect: (inst, cb) => {
      const se = inst.scrollElement as HTMLElement | null;
      inst.targetWindow ??= se?.ownerDocument?.defaultView ?? (typeof window !== 'undefined' ? window : null);
      return observeElementRect(inst, cb);
    },
    observeElementOffset: (inst, cb) => {
      const se = inst.scrollElement as HTMLElement | null;
      inst.targetWindow ??= se?.ownerDocument?.defaultView ?? (typeof window !== 'undefined' ? window : null);
      return observeElementOffset(inst, cb);
    },
    estimateSize: (i) => props.heights.estimate(props.settled()[i]?.turnId, DEFAULT_TURN_HEIGHT),
    getItemKey: (i) => props.settled()[i]?.turnId ?? `i${i}`,
    overscan: 4,
    initialRect: { width: 800, height: 600 },
    measureElement: (el, entry, instance) => {
      const h = defaultMeasure(el, entry, instance);
      const id = (el as HTMLElement).dataset?.turn;
      if (id) props.heights.set(id, h);
      return h;
    },
  });
  props.onVirt(virt);
  onCleanup(() => props.onVirt(undefined));
  if (import.meta.env.DEV) (window as unknown as Record<string, unknown>).__TL_VIRT__ = virt;

  return (
    <div class="tl-virtual" style={{ height: `${virt.getTotalSize()}px`, position: 'relative' }}>
      <Index each={virt.getVirtualItems()}>
        {(item) => {
          const turn = () => props.settled()[item().index];
          let node: HTMLDivElement | undefined;
          // The slot is reused for another turn — or the same turn's key moved
          // (a live turn settles under its real entry id): re-measure even when
          // the height happens to match, so the new key's row is real.
          createEffect(
            on(
              () => item().key,
              () => queueMicrotask(() => node && measureRow(virt, node)),
            ),
          );
          return (
            <div
              class="tl-vrow"
              data-index={item().index}
              data-turn={turn()?.turnId}
              ref={(el) => ((node = el), virt.measureElement(el))}
              style={{ position: 'absolute', top: 0, left: 0, width: '100%', transform: `translateY(${item().start}px)` }}
            >
              <Show when={turn()}>{(t) => <TurnRow turn={t()} ctx={props.ctx} last={!props.live() && item().index === props.settled().length - 1} />}</Show>
            </div>
          );
        }}
      </Index>
    </div>
  );
}

export { distanceToBottom };
