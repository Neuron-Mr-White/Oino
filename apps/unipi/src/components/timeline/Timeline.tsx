/**
 * The conversation timeline (WP-A6, §3.6 D-8; [chat-load] redesign — the full render
 * order is written down in oino-internal docs/dev/v2/design/chat-render-pipeline.md).
 *
 * - **Open**: until the tail chunk is whole (the store loads whole turns only, see
 *   `lib/session/chunks`) and fills two screens, the view shows the last-known turns
 *   (snapshot cache) or the skeleton; then the live turns replace it in ONE update, pinned
 *   to the bottom (or at the remembered anchor turn).
 * - **Rows**: settled turns render in normal document flow inside one list; only a window
 *   (viewport ± one screen) is mounted, the rest is the list's top / bottom padding, sized
 *   from the per-session height model (`lib/timeline/rowModel`: measured heights, else a
 *   per-kind estimate that never moves). Rows are keyed by turn id. In flow, rows cannot
 *   overlap and a row's growth pushes the ones below it like any page.
 * - **One writer**: every `scrollTop` write goes through the bottom lock (`lock.set`), from
 *   one function, `settle()`: pinned → keep the bottom; detached → keep the ANCHOR (the
 *   first visible turn, or the header the reader just toggled) at the same viewport offset.
 *   It runs synchronously after every change that can move rows (window shift, measured
 *   heights from the one ResizeObserver, an older chunk, the tail growing) — in the same
 *   frame, before paint; nothing else scrolls the timeline.
 * - **Bottom lock**: any reader gesture up detaches it at once; only the reader's own scroll
 *   back to the bottom or the jump button re-attaches it.
 * - **Older chunks**: only when the reader is near the top, not bottom-locked, not mid-flick
 *   (scroll idle), one chunk in flight, and one chunk per reader input; the "load earlier"
 *   row above the list has a fixed height, so it never moves anything.
 *
 * `Timeline` is keyed per session by `Conversation`; scroll memory lives per pane+session,
 * the height model per session (bounded LRU).
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { LoaderCircle, RefreshCw, TriangleAlert } from 'lucide-solid';
import { createBottomLock, distanceToBottom, SCROLL_IDLE_MS } from '../../lib/timeline/bottomLock';
import { createInnerFollow, INNER_SCROLLERS } from '../../lib/timeline/innerFollow';
import { indexAt, prefixOffsets, sessionHeights, windowOf } from '../../lib/timeline/rowModel';
import { readScrollMemory, writeScrollMemory, scrollMemoryKey } from '../../lib/timeline/scrollMemory';
import { readTimelineSnapshot, writeTimelineSnapshot } from '../../lib/timeline/snapshotCache';
import { EmptyState } from '../EmptyState';
import { LiveTail } from './LiveTail';
import { CommandReports } from './CommandReports';
import { showReportsInTimeline } from '../../lib/reports';
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
import { LOADED_MAX } from '../../lib/session/store';
import { blankReason, healBudget, HEAL_AFTER_MS } from '../../lib/timeline/chatHealth';
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

/** Open: older chunks loaded before the first paint at most (whole head turn / two screens). */
export const OPEN_FILL_MAX = 3;
/** Open: the view shows what it has after this long, whatever the fill did. */
const OPEN_TIMEOUT_MS = 4_000;
/** Rows mounted at most (a window of tiny rows on a tall screen). */
const WINDOW_MAX = 120;
/** Rows of the first window before any geometry exists. */
const INITIAL_ROWS = 12;
/** owner-fix-9: the gap between the viewport top and a message the user just sent. */
const TOP_OFFSET = 12;

const sameTurns = (a: readonly Turn[], b: readonly Turn[]) => a.length === b.length && a.every((t, i) => t === b[i]);
const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((t, i) => t === b[i]);

type Anchor = { id: string; top: number } | { el: HTMLElement; top: number };
type Target = { id: string; align: 'start' | 'center'; offset?: number };

export function Timeline(props: { ctx: TimelineCtx; dock?: JSX.Element; /** U10: the `header` slot band (scrolls with the transcript). */ header?: JSX.Element; /** U10: the `overlay` slot host (below the dock, §6.4). */ overlay?: JSX.Element; /** Z-87: the new-chat empty state (supplied by `Conversation`, which knows the host). */ empty?: JSX.Element; centeredEmpty?: boolean }) {
  const session = () => props.ctx.session;
  const liveTurns = () => session().turns();
  const cachedTurns = createMemo(() => readTimelineSnapshot(session().key));
  const heights = sessionHeights(session().key);
  const lock = createBottomLock();
  const [scrollEl, setScrollEl] = createSignal<HTMLDivElement>();
  const [overflowing, setOverflowing] = createSignal(false);
  const t0 = performance.now();
  let contentEl: HTMLDivElement | undefined;
  let listEl: HTMLDivElement | undefined;
  let tailEl: HTMLDivElement | undefined;
  let dockEl: HTMLDivElement | undefined;
  /** owner-fix-9: the room below the content that lets the sent message reach the top. */
  let roomEl: HTMLDivElement | undefined;

  // owner5 #6: while this chat is on screen, its command reports are rows here.
  createEffect(() => onCleanup(showReportsInTimeline(session().key)));

  // ── open phase: last-known turns / skeleton until the tail chunk is whole ──────
  const [ready, setReady] = createSignal(false);
  const turns = createMemo(() => (ready() ? liveTurns() : (cachedTurns() ?? [])));
  createEffect(() => {
    if (!ready()) return;
    const live = liveTurns();
    if (live.length > 0) writeTimelineSnapshot(session().key, live);
  });
  /** The running turn (outside the list); everything before it is the list. */
  const liveTurn = () => {
    const t = turns();
    const last = t[t.length - 1];
    return last?.live ? last : undefined;
  };
  const settled = createMemo(
    () => {
      const t = turns();
      return t.length && t[t.length - 1]!.live ? t.slice(0, -1) : t;
    },
    [],
    { equals: sameTurns },
  );
  const history = () => session().state.history;
  /** The first loaded turn is the newer part of a turn split by a page (only past the chunk cap or on a branch reload). */
  const headPartial = () => !!history()?.hasMore && settled()[0]?.kind === 'preamble';

  // ── row model + window ──────────────────────────────────────────────────────
  const index = createMemo(() => new Map(settled().map((t, i) => [t.turnId, i])));
  const [heightsRev, setHeightsRev] = createSignal(0);
  const offsets = createMemo(() => {
    heightsRev();
    return prefixOffsets(settled(), heights);
  });
  const [win, setWin] = createSignal<{ first?: string; last?: string }>({});
  const range = createMemo<[number, number]>(
    () => {
      const n = settled().length;
      if (!n) return [0, 0];
      const w = win();
      const a = w.first !== undefined ? index().get(w.first) : undefined;
      const b = w.last !== undefined ? index().get(w.last) : undefined;
      if (a === undefined || b === undefined || b < a) return [Math.max(0, n - INITIAL_ROWS), n];
      return [a, b + 1];
    },
    [0, 0],
    { equals: (x, y) => x[0] === y[0] && x[1] === y[1] },
  );
  const rowIds = createMemo(
    () => {
      const [a, b] = range();
      return settled()
        .slice(a, b)
        .map((t) => t.turnId);
    },
    [],
    { equals: sameIds },
  );
  const padTop = () => offsets()[range()[0]] ?? 0;
  const padBottom = () => {
    const o = offsets();
    return Math.max(0, (o[o.length - 1] ?? 0) - (o[range()[1]] ?? 0));
  };
  const rowNodes = new Map<string, HTMLDivElement>();

  const listTop = () => listEl?.offsetTop ?? 0;
  /**
   * The mounted rows' real heights into the model BEFORE the window moves: a row whose
   * content changed since the last ResizeObserver report would otherwise leave the list at
   * a stale height when it unmounts (the padding takes its place) and the view would jump.
   */
  const syncMounted = () => {
    let changed = false;
    for (const id of rowIds()) {
      const node = rowNodes.get(id);
      const i = index().get(id);
      const turn = i === undefined ? undefined : settled()[i];
      if (node?.isConnected && turn && heights.set(turn, node.getBoundingClientRect().height)) changed = true;
    }
    if (changed) {
      debug.measures++;
      setHeightsRev((n) => n + 1);
    }
  };
  /** Mounts the rows around the viewport (± one screen). Returns whether the window changed. */
  const computeWindow = (): boolean => {
    const el = scrollEl();
    const n = settled().length;
    if (!el || !listEl || !n) return false;
    syncMounted();
    const vh = el.clientHeight || 600;
    const over = Math.max(400, vh);
    const y = el.scrollTop - listTop();
    // A list shorter than the window itself mounts whole (no window to move).
    let [a, b] = (offsets()[n] ?? 0) <= vh + 2 * over ? [0, n] : windowOf(offsets(), y - over, y + vh + over);
    if (b - a > WINDOW_MAX) {
      const mid = Math.min(n - 1, indexAt(offsets(), y));
      a = Math.max(0, mid - WINDOW_MAX / 2);
      b = Math.min(n, a + WINDOW_MAX);
    }
    const first = settled()[a]!.turnId;
    const last = settled()[b - 1]!.turnId;
    const w = win();
    if (w.first === first && w.last === last) return false;
    setWin({ first, last });
    note('window', `${a}-${b}`, el.scrollTop);
    return true;
  };

  // ── the anchor + the one scroll writer ──────────────────────────────────────
  let anchor: Anchor | undefined;
  let target: Target | undefined;
  /** The header of the reader's own toggle: the anchor until the reader scrolls. */
  let toggled: HTMLElement | undefined;
  const debug = { settles: 0, restores: 0, maxRestore: 0, loads: 0, trims: 0, measures: 0 };
  /** DEV only: the last 48 scroll-relevant steps, for the stability rig's jump reports. */
  const trace: Array<[number, string, string | number, number]> = [];
  const note = (what: string, a: string | number, b: number) => {
    if (!import.meta.env.DEV) return;
    trace.push([Math.round(performance.now()), what, a, Math.round(b)]);
    if (trace.length > 48) trace.shift();
  };
  const viewTop = (el: HTMLElement) => el.getBoundingClientRect().top;
  const capture = () => {
    const el = scrollEl();
    if (!el) return;
    const top = viewTop(el);
    if (toggled?.isConnected) {
      anchor = { el: toggled, top: toggled.getBoundingClientRect().top - top };
      return;
    }
    toggled = undefined;
    for (const id of rowIds()) {
      const node = rowNodes.get(id);
      if (!node) continue;
      const r = node.getBoundingClientRect();
      if (r.bottom > top + 1) {
        anchor = { id, top: r.top - top };
        note('capture', id, anchor.top);
        return;
      }
    }
    anchor = undefined;
  };
  const anchorNode = (a: Anchor) => ('el' in a ? a.el : rowNodes.get(a.id));
  const restore = () => {
    const el = scrollEl();
    if (!el || !anchor) return;
    const node = anchorNode(anchor);
    if (!node?.isConnected) return;
    const moved = node.getBoundingClientRect().top - viewTop(el) - anchor.top;
    if (Math.abs(moved) < 0.5) return;
    debug.restores++;
    note('restore', 'id' in anchor ? anchor.id : 'el', moved);
    debug.maxRestore = Math.max(debug.maxRestore, Math.abs(moved));
    lock.set(el, el.scrollTop + moved);
  };
  const placeTarget = (t: Target) => {
    const el = scrollEl();
    const i = index().get(t.id);
    if (!el || i === undefined) return;
    if (!rowNodes.has(t.id)) {
      const s = settled();
      setWin({ first: s[Math.max(0, i - 2)]!.turnId, last: s[Math.min(s.length - 1, i + 6)]!.turnId });
    }
    const node = rowNodes.get(t.id);
    if (!node) return;
    const r = node.getBoundingClientRect();
    const want = t.align === 'center' ? Math.max(0, (el.clientHeight - r.height) / 2) : (t.offset ?? 0);
    lock.set(el, el.scrollTop + (r.top - viewTop(el)) - want);
    anchor = { id: t.id, top: want };
  };
  // ── owner-fix-9: the message the user just sent sits at the top of the viewport ──
  /** A sent prompt is being kept at the top (until its turn's content fills the screen). */
  let topOn = false;
  /** That prompt's turn, once the host echoed it (live, then a settled row). */
  let topTurn: string | undefined;
  const setRoom = (px: number) => {
    if (!roomEl) return;
    const h = `${Math.max(0, Math.round(px))}px`;
    if (roomEl.style.height !== h) roomEl.style.height = h;
  };
  const endTop = () => {
    topOn = false;
    topTurn = undefined;
    setRoom(0);
    lock.releaseTop();
  };
  const startTop = () => {
    topOn = true;
    topTurn = undefined;
    toggled = undefined;
    target = undefined;
    lock.enterTop();
  };
  /** The sent message's node: its settled row, else the live tail (outbox bubble / live turn), else the last row. */
  const topNode = (): HTMLElement | undefined => {
    const row = topTurn ? rowNodes.get(topTurn) : undefined;
    if (row?.isConnected) return row;
    if (tailEl && tailEl.offsetHeight > 0) return tailEl;
    const s = settled();
    const last = s[s.length - 1];
    return last ? rowNodes.get(last.turnId) : undefined;
  };
  /**
   * Sizes the room below the content so the sent message can sit TOP_OFFSET px below the
   * viewport top, and (in the lock's `top` mode) scrolls it there. Once the content from
   * the message down fills the screen, the room goes and the normal bottom-follow resumes.
   */
  const placeTop = (el: HTMLElement, scroll: boolean) => {
    const node = topNode();
    if (!node || !roomEl) return endTop();
    const r = node.getBoundingClientRect();
    const avail = el.clientHeight - (dockEl?.offsetHeight ?? 0);
    const below = roomEl.getBoundingClientRect().top - r.top;
    if (below >= avail - TOP_OFFSET) {
      const wasTop = lock.mode() === 'top';
      endTop();
      if (wasTop) lock.jump(el);
      return;
    }
    setRoom(avail - TOP_OFFSET - below);
    if (scroll) lock.set(el, el.scrollTop + (node.getBoundingClientRect().top - viewTop(el)) - TOP_OFFSET);
  };
  const syncOverflow = () => {
    const el = scrollEl();
    if (el) setOverflowing(el.scrollHeight - el.clientHeight > 4);
  };
  /**
   * The one place that scrolls: pin (locked) or keep the anchor (detached), mount the
   * window for the new position, repeat until the window is stable (bounded).
   */
  const settle = () => {
    settleQueued = false;
    const el = scrollEl();
    if (!el) return;
    debug.settles++;
    for (let i = 0; i < 4; i++) {
      const t = target;
      target = undefined;
      if (topOn) placeTop(el, !t && lock.mode() === 'top');
      if (t) placeTarget(t);
      else if (lock.mode() === 'top') {
        // placed above
      } else if (lock.locked()) lock.pin(el);
      else restore();
      if (!computeWindow()) break;
    }
    lock.reconcile(el);
    if (!lock.locked()) capture();
    syncOverflow();
    updateActive();
  };
  let settleQueued = false;
  /** From reactive code (store effects): the DOM is updated, settle right after (same task, before paint). */
  const scheduleSettle = () => {
    if (settleQueued) return;
    settleQueued = true;
    queueMicrotask(settle);
  };

  // ── open: fill, then reveal in one update ────────────────────────────────────
  const memKey = () => scrollMemoryKey('main', props.ctx.hostId, props.ctx.sessionId());
  const saved = readScrollMemory(memKey());
  // onCleanup must not read props.ctx: the parent's memos are already disposed
  // when this component is cleaned up (Solid cleanNode crash on route change).
  let lastMemKey = memKey();
  createEffect(() => {
    lastMemKey = memKey();
  });
  let fills = 0;
  const finishOpen = () => {
    if (ready()) return;
    setReady(true);
    const at = saved && 'anchor' in saved && saved.anchor ? saved.anchor : undefined;
    if (at && index().has(at.turnId)) {
      lock.detach();
      target = { id: at.turnId, align: 'start', offset: at.offset };
    }
    queueMicrotask(() => {
      settle();
      requestAnimationFrame(() => requestAnimationFrame(() => perf('tl:first-paint', t0)));
    });
  };
  createEffect(() => {
    if (ready()) return;
    const st = session().state;
    const live = liveTurns();
    if (st.conn !== 'live') {
      // A handle that already holds turns (reconnecting) shows them as they are.
      if (live.length > 0) finishOpen();
      return;
    }
    const h = st.history;
    if (h?.loading) return;
    const fill = () => {
      if (!h?.hasMore || h.error || fills >= OPEN_FILL_MAX || !session().loadEarlier) return false;
      const t = live.length && live[live.length - 1]!.live ? live.slice(0, -1) : live;
      if (t[0]?.kind === 'preamble') return true;
      const vh = scrollEl()?.clientHeight || 800;
      return prefixOffsets(t, heights)[t.length]! < 2 * vh;
    };
    if (fill()) {
      fills++;
      void session().loadEarlier!();
      if (session().state.history?.loading) return;
    }
    finishOpen();
  });
  onMount(() => {
    const timer = setTimeout(() => session().state.conn === 'live' && finishOpen(), OPEN_TIMEOUT_MS);
    onCleanup(() => clearTimeout(timer));
  });
  onCleanup(() => {
    const el = scrollEl();
    if (!el || !ready()) return;
    // The anchor is current (every settle and reader scroll keeps it); the rows may already be gone.
    if (lock.locked()) writeScrollMemory(lastMemKey, { bottomLocked: true });
    else {
      writeScrollMemory(lastMemKey, anchor && 'id' in anchor ? { scrollTop: el.scrollTop, anchor: { turnId: anchor.id, offset: anchor.top } } : { scrollTop: el.scrollTop });
    }
  });

  // ── older chunks: near the top, detached, idle, one per reader input ─────────
  let readerSinceLoad = false;
  /** The last reader input event (wheel / touch / key / scrollbar press). */
  let lastInput = Number.NEGATIVE_INFINITY;
  let lastTop = 0;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  const canLoad = () => {
    const h = history();
    return !!h?.hasMore && !h.loading && !h.error && session().state.conn === 'live' && !!session().loadEarlier && ready();
  };
  const maybeLoadEarlier = () => {
    const el = scrollEl();
    if (!el || !canLoad()) return;
    if (headPartial()) {
      debug.loads++;
      void session().loadEarlier!();
      return;
    }
    if (!readerSinceLoad) return;
    const fits = el.scrollHeight - el.clientHeight <= 4;
    if (lock.locked() && !fits) return;
    if (lock.scrolling()) return queueIdleCheck();
    if (!fits && el.scrollTop - listTop() > 1.5 * el.clientHeight) return;
    readerSinceLoad = false;
    debug.loads++;
    void session().loadEarlier!();
  };
  const queueIdleCheck = () => {
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      idleTimer = undefined;
      maybeLoadEarlier();
    }, SCROLL_IDLE_MS + 20);
  };
  onCleanup(() => idleTimer && clearTimeout(idleTimer));
  // A split head (branch reload, chunk cap) is completed at once, above the reader.
  createEffect(() => {
    if (ready() && headPartial() && !history()?.loading) queueMicrotask(maybeLoadEarlier);
  });

  // perf-app: back at the bottom of a chat read far up, the old chunks go (bounded parsed
  // state per chat; they load again on the way up). Only while locked — chunks only load
  // while detached, so trim and load can never feed each other.
  const maybeTrim = () => {
    if (!ready() || !lock.locked() || history()?.loading || session().state.entries.length <= LOADED_MAX) return;
    if (session().trimLoaded?.(LOADED_MAX / 2)) debug.trims++;
  };
  if (import.meta.env.DEV || import.meta.env.MODE === 'test') {
    (window as unknown as Record<string, unknown>).__TL_PAGING__ = () => ({ ...debug, rows: rowIds().length, locked: lock.locked(), ready: ready() });
    (window as unknown as Record<string, unknown>).__TL_TRACE__ = () => trace.slice();
  }

  // ── wiring: reader input, scroll, observers ─────────────────────────────────
  /** The reader's own accordion toggle: its header holds still while the block below it grows. */
  const onContentClick = (e: MouseEvent) => {
    const hit = (e.target as HTMLElement | null)?.closest?.('[aria-expanded], [data-testid="tl-bubble-fold"], [data-testid="turn-files-toggle"]') as HTMLElement | null | undefined;
    if (!hit || tailEl?.contains(hit) || !scrollEl()) return;
    lock.detach();
    toggled = hit;
    capture();
  };
  onMount(() => {
    const el = scrollEl();
    if (!el) return;
    const reader = () => {
      lastInput = performance.now();
      readerSinceLoad = true;
      toggled = undefined;
      target = undefined;
    };
    const up = () => {
      reader();
      capture();
      lock.gestureUp(el);
      queueIdleCheck();
    };
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) up();
      else reader();
    };
    let touchY: number | undefined;
    const onTouchStart = (e: TouchEvent) => {
      touchY = e.touches[0]?.clientY;
    };
    const onTouchMove = (e: TouchEvent) => {
      const y = e.touches[0]?.clientY;
      if (y === undefined || touchY === undefined) return;
      if (y > touchY + 2) up();
      else reader();
      touchY = y;
    };
    const onPointerDown = (e: PointerEvent) => {
      // Scrollbar track/thumb press on the scroll container itself is a reader gesture;
      // clicking buttons/accordions inside `.tl-content` or `.tl-dock` is not.
      if (e.target === el) up();
    };
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.closest?.('input, textarea, [contenteditable="true"]')) return;
      if (['PageUp', 'ArrowUp', 'Home'].includes(e.key)) up();
      else if (['PageDown', 'ArrowDown', 'End', ' '].includes(e.key)) reader();
    };
    el.addEventListener('wheel', onWheel, { passive: true });
    el.addEventListener('touchstart', onTouchStart, { passive: true });
    el.addEventListener('touchmove', onTouchMove, { passive: true });
    el.addEventListener('pointerdown', onPointerDown, { passive: true });
    el.addEventListener('keydown', onKey);
    onCleanup(() => {
      el.removeEventListener('wheel', onWheel);
      el.removeEventListener('touchstart', onTouchStart);
      el.removeEventListener('touchmove', onTouchMove);
      el.removeEventListener('pointerdown', onPointerDown);
      el.removeEventListener('keydown', onKey);
    });

    // Store-driven changes (entries, live tail, outbox, new turns, older chunks): settle
    // after the DOM update. Only entries appended at the END (or a new outbox message) are
    // news for the unread dot: older chunks prepend (the last entry stays), a trim drops
    // the top, branch navigation moves the leaf.
    let seenLen = -1;
    let seenLast: string | undefined;
    let seenOutbox = 0;
    createEffect(() => {
      session().state.entriesRev;
      liveTurn();
      settled();
      history()?.hasMore;
      const entries = session().state.entries;
      const len = entries.length;
      const last = entries[len - 1]?.id;
      const ob = session().state.outbox;
      const outbox = ob.length;
      const live = liveTurn();
      if (!ready()) return;
      // owner-fix-9: a prompt the user just sent from an idle chat goes to the top.
      if (seenLen >= 0 && outbox > seenOutbox && ob[outbox - 1]?.mode === 'prompt' && ob[outbox - 1]?.status === 'sending' && !live) startTop();
      if (topOn && !topTurn && live) topTurn = live.turnId;
      if (seenLen >= 0 && ((len > seenLen && last !== seenLast) || outbox > seenOutbox)) lock.arrived();
      seenLen = len;
      seenLast = last;
      seenOutbox = outbox;
      scheduleSettle();
    });

    // Output inside an expanded, height-capped tail accordion: the box follows its own
    // bottom (the timeline does not grow once it is capped, so this is the only signal) —
    // and it is news while detached.
    const inner = createInnerFollow();
    let sweepQueued = false;
    const sweepInner = () => {
      sweepQueued = false;
      if (!tailEl || !ready()) return;
      if (inner.sweep(tailEl.querySelectorAll<HTMLElement>(INNER_SCROLLERS))) lock.arrived();
    };
    /** An image in the tail just decoded: its growth is layout, not news. */
    let imageUntil = 0;
    if (tailEl && typeof MutationObserver !== 'undefined') {
      const mo = new MutationObserver(() => {
        if (sweepQueued) return;
        sweepQueued = true;
        requestAnimationFrame(sweepInner);
      });
      mo.observe(tailEl, { childList: true, subtree: true, characterData: true });
      const onLoad = (e: Event) => {
        if ((e.target as HTMLElement | null)?.tagName === 'IMG') imageUntil = performance.now() + 150;
      };
      tailEl.addEventListener('load', onLoad, true);
      onCleanup(() => {
        mo.disconnect();
        tailEl?.removeEventListener('load', onLoad, true);
      });
    }

    if (typeof ResizeObserver !== 'undefined') {
      // Any content height change (rows measured, the tail streaming, a fold, the earlier
      // row): settle in the same frame — pin while locked, else keep the anchor.
      const grow = new ResizeObserver(() => settle());
      if (contentEl) grow.observe(contentEl);
      // [working-line] the dock growing (the working line easing in, a widget) re-settles too: a pinned chat stays pinned.
      if (dockEl) grow.observe(dockEl);
      // The live tail growing (stream, new tool rows, a subagent card) is the "new output"
      // the dot reports; width changes reflow it (layout).
      let tailH = -1;
      let tailW = -1;
      const tail = new ResizeObserver(() => {
        if (!tailEl) return;
        const h = tailEl.offsetHeight;
        const w = tailEl.offsetWidth;
        const news = ready() && tailH >= 0 && w === tailW && h > tailH + 0.5 && performance.now() > imageUntil;
        tailH = h;
        tailW = w;
        if (news) lock.arrived();
      });
      if (tailEl) tail.observe(tailEl);
      const view = new ResizeObserver(() => settle());
      view.observe(el);
      onCleanup(() => {
        grow.disconnect();
        tail.disconnect();
        view.disconnect();
      });
    }
  });
  /** Row heights: one observer for the window's rows, applied as one batch per frame. */
  const rowRO =
    typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver((entries) => {
          let changed = false;
          for (const e of entries) {
            const node = e.target as HTMLDivElement;
            const id = node.dataset.turn;
            const i = id === undefined ? undefined : index().get(id);
            const turn = i === undefined ? undefined : settled()[i];
            if (!turn || !node.isConnected) continue;
            const h = e.borderBoxSize?.[0]?.blockSize ?? node.offsetHeight;
            if (heights.set(turn, h)) changed = true;
          }
          if (changed) {
            debug.measures++;
            setHeightsRev((n) => n + 1);
          }
          note('ro', entries.length, changed ? 1 : 0);
          settle();
        })
      : undefined;
  onCleanup(() => rowRO?.disconnect());

  const onScroll = () => {
    const el = scrollEl();
    if (!el) return;
    const top = el.scrollTop;
    const moved = top - lastTop;
    lastTop = top;
    note('scroll', moved, top);
    if (lock.onScroll(el)) {
      // The browser clamped the offset (content below shrank — a fold closed near the end):
      // geometry, not the reader. The anchor follows it; it is no reason to load anything.
      const clamp = moved < 0 && distanceToBottom(el) <= 1 && performance.now() - lastInput > 600;
      if (!clamp) {
        // The reader moved: the anchor is wherever they are now.
        readerSinceLoad = true;
        toggled = undefined;
        target = undefined;
      }
      capture();
      settle();
      if (!clamp) queueIdleCheck();
      if (lock.locked() && !clamp) maybeTrim();
    } else {
      syncOverflow();
      updateActive();
    }
  };

  // ── live tail → list migration: pre-seed its measured height ─────
  createEffect(
    on(liveTurn, (cur, prev) => {
      if (prev && !cur && heights.get(prev.turnId) === undefined && tailEl?.offsetHeight) heights.set(prev, tailEl.offsetHeight);
    }),
  );

  // chat-vanish: with no turns every non-live state draws something — a reconnect is a
  // skeleton (under its banner), a stream that ended or gave up is the error card with
  // Retry. `closed` without an error (an evicted handle) and `lost` used to draw nothing.
  const openFailed = () => {
    const st = session().state;
    if (turns().length > 0 || st.banner?.kind === 'taken_over') return false;
    return st.conn === 'lost' || st.conn === 'closed';
  };
  const loading = () => turns().length === 0 && !openFailed() && (!ready() || session().state.conn === 'connecting' || session().state.conn === 'reconnecting');
  const empty = () => ready() && session().state.conn === 'live' && turns().length === 0 && !session().state.outbox.length;
  const retryOpen = () => (props.ctx.heal ? props.ctx.heal('manual') : session().retry());

  // ── self-heal (chat-vanish): blank while the host is live → reopen / remount, logged ──
  const verdict = createMemo(() => {
    const s = session();
    const st = s.state;
    return blankReason({
      hostOnline: props.ctx.hostOnline?.() ?? true,
      handleClosed: !!s.closed && st.conn === 'closed',
      conn: st.conn,
      connErrorCode: st.connError?.code,
      takenOver: st.banner?.kind === 'taken_over',
      archived: s.key.startsWith('archived:'),
      settled: settled().length,
      rows: scrollEl() ? rowIds().length : 1,
    });
  });
  const budget = healBudget();
  createEffect(
    on(verdict, (reason) => {
      if (!reason || !props.ctx.heal) return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const check = () => {
        timer = undefined;
        if (verdict() !== reason) return;
        if (!budget.allow()) return; // the error card's Retry from here on
        // A 0-height window (hidden pane, a resize mid-layout): lay out again first.
        if (reason === 'no_rows') settle();
        props.ctx.heal!(reason);
        timer = setTimeout(check, HEAL_AFTER_MS);
      };
      timer = setTimeout(check, HEAL_AFTER_MS);
      onCleanup(() => timer && clearTimeout(timer));
    }),
  );

  // ── navigator (Z-25): the query at the top of the viewport is active ─
  const [activeTurn, setActiveTurn] = createSignal<string>();
  function updateActive() {
    const el = scrollEl();
    if (!el) return;
    const o = offsets();
    const n = settled().length;
    const y = el.scrollTop - listTop() + 2;
    const i = n && y < (o[n] ?? 0) ? indexAt(o, Math.max(0, y)) : n;
    setActiveTurn(settled()[i]?.turnId ?? liveTurn()?.turnId);
  }

  /** Navigator jump: mount the turn, land it at the top of the viewport, detach (Z-25). */
  const jumpToTurn = (turnId: string) => {
    const el = scrollEl();
    if (!el) return;
    lock.detach();
    lock.seen();
    toggled = undefined;
    if (index().has(turnId)) target = { id: turnId, align: 'start' };
    else if (tailEl) {
      lock.set(el, tailEl.offsetTop);
      capture();
    }
    settle();
  };

  // ── find in chat (WP-A18, Z-79) ─────────────────────────────────
  const findIdx = createMemo(() => buildFindIndex(turns(), findState().query));
  /** The hit entry id's turn (whole-session results jump through this). */
  const turnOfEntry = (entryId: string) => turns().find((t) => t.entries.some((e) => e.id === entryId))?.turnId;
  onMount(() => {
    setFindAdapter({ jumpToTurn });
    onCleanup(() => setFindAdapter(undefined));
  });
  // Rows mount/unmount with the window: re-highlight what is mounted.
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
  const showTarget = (root: HTMLElement, idx: FindIndex, active: number, scroll: boolean) => {
    const hit = locateMatch(idx, active);
    if (!hit) return;
    if (activateFindMark(root, hit.turnId, hit.occurrence, scroll)) return;
    // The turn is outside the window: mount + centre it, then mark.
    if (!scroll || !index().has(hit.turnId)) return;
    lock.detach();
    toggled = undefined;
    target = { id: hit.turnId, align: 'center' };
    settle();
    const fresh = findIdx();
    clearFindHighlights(root);
    applyFindHighlights(root, fresh.query);
    const again = locateMatch(fresh, findState().active);
    if (again) activateFindMark(root, again.turnId, again.occurrence, true);
  };
  // The query/open/active changed: redo the highlights from scratch.
  createEffect(() => {
    const root = contentEl;
    const s = findState();
    const idx = findIdx();
    if (!root) return;
    clearFindHighlights(root);
    if (!s.open || !idx.query) return;
    applyFindHighlights(root, idx.query);
    queueMicrotask(() => showTarget(root, idx, s.active, true));
  });
  // New rows mounted under the marks: highlight just those.
  createEffect(() => {
    findDomRev();
    const root = contentEl;
    const s = findState();
    const idx = findIdx();
    if (!root || !s.open || !idx.query) return;
    applyFindHighlights(root, idx.query);
    showTarget(root, idx, s.active, false);
  });
  // Leaving the conversation closes find (one surface, scoped to the chat).
  onCleanup(() => closeFind());

  return (
    <div class="tl-outer">
      <FindBar session={props.ctx.session} index={findIdx} turnOf={turnOfEntry} jump={jumpToTurn} />
      <TurnNavigator turns={turns} width={props.ctx.columnWidth} activeId={activeTurn} onJump={jumpToTurn} />
      <SelectionQuote root={scrollEl} sessionKey={() => props.ctx.session.key} />
      <TurnMenus ctx={props.ctx} />
      <div class="tl" ref={setScrollEl} data-testid="tl" data-ready={ready() ? '' : undefined} data-centered-empty={props.centeredEmpty && empty() ? '' : undefined} onScroll={onScroll}>
        <div class="tl-content" ref={contentEl} onClick={onContentClick}>
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
                body={session().state.connError?.message || (session().state.conn === 'lost' ? 'Lost the connection to this chat.' : 'The connection to this chat closed.')}
                testId="tl-open-failed-card"
                action={
                  <button type="button" class="btn btn-secondary btn-sm" data-testid="tl-open-retry" onClick={retryOpen}>
                    <RefreshCw size={14} /> Retry
                  </button>
                }
              />
            </div>
          </Show>
          <Show when={empty() && !props.centeredEmpty}>
            <div class="tl-empty" data-testid="tl-empty">
              {props.empty ?? <p>Say hi — the first message starts the session.</p>}
            </div>
          </Show>
          <Show when={ready() && history()?.hasMore && turns().length > 0}>
            <EarlierRow loading={!!history()?.loading} error={history()?.error} onLoad={() => void session().loadEarlier?.()} />
          </Show>
          <div class="tl-list" ref={listEl} data-testid="tl-list" style={{ 'padding-top': `${padTop()}px`, 'padding-bottom': `${padBottom()}px` }}>
            <For each={rowIds()}>
              {(id) => {
                const at = () => index().get(id);
                const turn = () => {
                  const i = at();
                  return i === undefined ? undefined : settled()[i];
                };
                return (
                  <div
                    class="tl-vrow"
                    data-turn={id}
                    data-index={at()}
                    ref={(node) => {
                      rowNodes.set(id, node);
                      rowRO?.observe(node);
                      onCleanup(() => {
                        rowRO?.unobserve(node);
                        if (rowNodes.get(id) === node) rowNodes.delete(id);
                      });
                    }}
                  >
                    <Show when={turn()}>{(t) => <TurnRow turn={t()} ctx={props.ctx} last={!liveTurn() && at() === settled().length - 1} />}</Show>
                  </div>
                );
              }}
            </For>
          </div>
          <div ref={tailEl} class="tl-tail-wrap">
            <Show when={ready()}>
              <LiveTail live={liveTurn()} ctx={props.ctx} />
            </Show>
            <CommandReports sessionKey={session().key} />
          </div>
          <div ref={roomEl} class="tl-top-room" aria-hidden="true" data-testid="tl-top-room" />
        </div>
        <div ref={dockEl} class="tl-dock" data-testid="tl-dock">
          <Show when={actionNotice()}>
            <div class="tl-notice" role="status" data-testid="tl-notice">
              {actionNotice()}
            </div>
          </Show>
          <JumpButton show={lock.mode() === 'detached' && overflowing()} unread={lock.unread()} onJump={() => scrollEl() && (lock.jump(scrollEl()!), settle())} />
          <Show when={props.centeredEmpty && empty()}><div class="empty-chat" data-variant="start">{props.empty}</div></Show>
          {props.dock}
          <div class="tl-overlay-host" data-testid="tl-overlay-host">
            {props.overlay}
          </div>
        </div>
      </div>
    </div>
  );
}

/** The "load earlier" row above the oldest loaded turn (fixed height: it never moves the rows). */
function EarlierRow(props: { loading: boolean; error?: string; onLoad: () => void }) {
  return (
    <div class="tl-earlier" data-testid="tl-earlier" aria-live="polite">
      <Show
        when={!props.error}
        fallback={
          <button type="button" class="tl-earlier-btn" data-testid="tl-earlier-retry" onClick={() => props.onLoad()}>
            <RefreshCw size={14} /> Couldn't load earlier messages · Retry
          </button>
        }
      >
        <button type="button" class="tl-earlier-btn" data-testid="tl-earlier-load" disabled={props.loading} onClick={() => props.onLoad()}>
          <Show when={props.loading} fallback={<>Load earlier messages</>}>
            <LoaderCircle size={14} class="tl-earlier-spin" /> Loading earlier messages…
          </Show>
        </button>
      </Show>
    </div>
  );
}

export { distanceToBottom };
