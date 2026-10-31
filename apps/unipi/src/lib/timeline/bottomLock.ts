/**
 * Bottom lock (app spec §3.6 Z-23/Z-24, WP-A6; ZCode
 * `chat-scroll-bottom-pinning.md`): the single follow state of a timeline.
 * `locked` is the user's *intent*, not the instantaneous geometry — layout
 * work (stream growth, folds, measureElement, virtualizer compensation)
 * never detaches and never re-locks on its own.
 *
 *   wheel / touch / keyboard / scrollbar intent + distance > 48 px  → DETACHED
 *   user scroll back within 48 px of the bottom, or a jump          → LOCKED
 *   content not overflowing                                          → lockable (nothing to lose)
 *   jump button                                                      → detached AND overflowing only
 *
 * While locked the timeline writes the new bottom `scrollTop` in the same
 * frame the content grew; while detached, growth only sets the unread dot.
 */
import { createSignal } from 'solid-js';

/** How far from the bottom still counts as "at the bottom" (ZCode's 48 px). */
export const BOTTOM_PX = 48;
/** Real overflow needed before detaching is meaningful (sub-pixel noise). */
const OVERFLOW_PX = 4;
/** How long after a user input a scroll event still counts as theirs. */
const INTENT_MS = 450;

/** What the scroller needs to expose (the DOM element satisfies it). */
export type Scroller = {
  scrollTop: number;
  readonly scrollHeight: number;
  readonly clientHeight: number;
};

export function distanceToBottom(el: Scroller): number {
  return el.scrollHeight - el.clientHeight - el.scrollTop;
}

export type BottomLock = {
  /** Currently following the tail. */
  readonly locked: () => boolean;
  readonly detached: () => boolean;
  /** New content arrived while detached (the jump button's dot, Z-24). */
  readonly unread: () => boolean;
  /** Marks a user scroll input (wheel/touch/key/scrollbar): the next scroll events are theirs. */
  intent(): void;
  /** The scroll handler: resolves intent vs layout, updates `locked`/`unread`. */
  onScroll(el: Scroller): void;
  /** Content grew (stream, fold, measure): pin when locked, flag unread when detached. */
  grew(el: Scroller): void;
  /** Content shrank or the viewport grew: re-check geometry (never unlocks by itself). */
  reconcile(el: Scroller): void;
  /** The jump button / send / session-open pin: lock + jump to the bottom instantly. */
  jump(el: Scroller): void;
  /** Keeps the bottom while locked (same frame as the growth). */
  pin(el: Scroller): void;
  /** Locks without scrolling (initial state, restore). */
  lock(): void;
  /** Detaches without scrolling (positional jumps: find, navigator, fork). */
  detach(): void;
  /** Clears the unread dot. */
  seen(): void;
};

export function createBottomLock(): BottomLock {
  const [locked, setLocked] = createSignal(true);
  const [unread, setUnread] = createSignal(false);
  let intentUntil = 0;
  const intends = () => Date.now() <= intentUntil;

  const pin = (el: Scroller) => {
    el.scrollTop = el.scrollHeight - el.clientHeight;
  };

  return {
    locked,
    detached: () => !locked(),
    unread,
    intent() {
      intentUntil = Date.now() + INTENT_MS;
    },
    onScroll(el) {
      const d = distanceToBottom(el);
      if (intends()) {
        // A real user scroll: intent beats layout.
        if (d > BOTTOM_PX && el.scrollHeight - el.clientHeight > OVERFLOW_PX) setLocked(false);
        else if (d <= BOTTOM_PX) {
          setLocked(true);
          setUnread(false);
        }
        return;
      }
      // No user input behind this event: geometry bookkeeping only.
      // Reaching the bottom re-engages (e.g. the user's own scroll already
      // settled), and a non-overflowing view has nothing to preserve.
      if (d <= BOTTOM_PX || el.scrollHeight - el.clientHeight <= OVERFLOW_PX) {
        if (!locked()) {
          setLocked(true);
          setUnread(false);
        }
      }
    },
    grew(el) {
      if (locked()) pin(el);
      else setUnread(true);
    },
    reconcile(el) {
      // A view that cannot scroll cannot hold a reading position (ZCode rule).
      if (el.scrollHeight - el.clientHeight <= OVERFLOW_PX) {
        setLocked(true);
        setUnread(false);
      }
    },
    jump(el) {
      intentUntil = 0;
      setLocked(true);
      setUnread(false);
      pin(el);
    },
    pin,
    lock: () => setLocked(true),
    detach: () => setLocked(false),
    seen: () => setUnread(false),
  };
}
