/**
 * Bottom lock (app spec §3.6 Z-23/Z-24; [chat-load] rules, design
 * oino-internal docs/dev/v2/design/chat-render-pipeline.md §New.d): the single follow
 * state of a timeline, and the ONLY writer of the scroller's `scrollTop` (every write goes
 * through `set`, so a scroll event can tell the reader's scrolling from ours).
 *
 *   any reader scroll gesture up (wheel / touch / key / scrollbar), or a reader scroll
 *   that moved the view up                                          → DETACHED at once
 *   the reader's own scroll reaching the bottom, or the jump button  → LOCKED
 *   a view that cannot scroll                                        → locked (nothing to keep)
 *   nothing else re-attaches: no layout change, no row measure, no older chunk.
 *
 * While locked the timeline pins the bottom in the frame the content grew. The unread dot
 * is NOT geometry: only `arrived()` (new entries / tail growth after the detach) sets it.
 */
import { createSignal } from 'solid-js';

/** Within this many px of the bottom a reader's downward scroll re-attaches. */
export const BOTTOM_PX = 8;
/** Real overflow needed before detaching is meaningful (sub-pixel noise). */
const OVERFLOW_PX = 4;
/** A reader scroll this recent means the view is moving (flick / inertia): no chunk loads. */
export const SCROLL_IDLE_MS = 160;

/** What the scroller needs to expose (the DOM element satisfies it). */
export type Scroller = {
  scrollTop: number;
  readonly scrollHeight: number;
  readonly clientHeight: number;
};

export function distanceToBottom(el: Scroller): number {
  return el.scrollHeight - el.clientHeight - el.scrollTop;
}

const overflows = (el: Scroller) => el.scrollHeight - el.clientHeight > OVERFLOW_PX;

export type BottomLock = {
  /** Currently following the tail. */
  readonly locked: () => boolean;
  readonly detached: () => boolean;
  /** New content arrived while detached (the jump button's dot, Z-24). */
  readonly unread: () => boolean;
  /** The one programmatic write (`scrollTop = top`); remembered so its scroll event is ours. */
  set(el: Scroller, top: number): void;
  /** A reader gesture that scrolls UP (wheel up, touch drag down, PageUp/↑/Home, scrollbar grab). */
  gestureUp(el: Scroller): void;
  /** The scroll handler: `true` when the event was the reader's (not one of our writes). */
  onScroll(el: Scroller): boolean;
  /** The reader scrolled within the last `SCROLL_IDLE_MS` (flick / inertia in progress). */
  scrolling(): boolean;
  /** Content height changed: pin when locked; never flags unread. */
  grew(el: Scroller): void;
  /** New content arrived at the end (entries / tail growth): the unread dot while detached. */
  arrived(): void;
  /** A view that cannot scroll holds no reading position: lock (and show it from the top). */
  reconcile(el: Scroller): void;
  /** The jump button / send / session-open pin: lock + jump to the bottom instantly. */
  jump(el: Scroller): void;
  /** Keeps the bottom while locked (same frame as the growth). */
  pin(el: Scroller): void;
  /** Detaches without scrolling (positional jumps: find, navigator, a reader's toggle). */
  detach(): void;
  /** Clears the unread dot. */
  seen(): void;
};

export function createBottomLock(now: () => number = () => performance.now()): BottomLock {
  const [locked, setLocked] = createSignal(true);
  const [unread, setUnread] = createSignal(false);
  /** The value our last write left (`NaN` = none pending). */
  let ours = Number.NaN;
  let prevTop = 0;
  let lastReader = Number.NEGATIVE_INFINITY;

  const set = (el: Scroller, top: number) => {
    el.scrollTop = top;
    ours = el.scrollTop;
    prevTop = ours;
  };
  const pin = (el: Scroller) => set(el, Math.max(0, el.scrollHeight - el.clientHeight));
  const attach = () => {
    setLocked(true);
    setUnread(false);
  };

  return {
    locked,
    detached: () => !locked(),
    unread,
    set,
    gestureUp(el) {
      lastReader = now();
      if (overflows(el)) setLocked(false);
    },
    onScroll(el) {
      const top = el.scrollTop;
      if (Math.abs(top - ours) < 1) {
        prevTop = top;
        return false;
      }
      ours = Number.NaN;
      lastReader = now();
      const d = distanceToBottom(el);
      const up = top < prevTop - 0.5;
      prevTop = top;
      if (up && d > 1 && overflows(el)) setLocked(false);
      else if (!up && d <= BOTTOM_PX && !locked()) attach();
      return true;
    },
    scrolling: () => now() - lastReader < SCROLL_IDLE_MS,
    grew(el) {
      if (locked()) pin(el);
    },
    arrived() {
      if (!locked()) setUnread(true);
    },
    reconcile(el) {
      if (overflows(el)) return;
      if (!locked()) attach();
      // F-12: content that fits shows from the top — a stale offset left the first bubble cut off above empty space.
      if (el.scrollTop > 0) set(el, 0);
    },
    jump(el) {
      attach();
      pin(el);
    },
    pin,
    detach: () => setLocked(false),
    seen: () => setUnread(false),
  };
}
