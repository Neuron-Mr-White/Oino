import { describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { BOTTOM_PX, createBottomLock, distanceToBottom, SCROLL_IDLE_MS, type Scroller } from './bottomLock';

/** A scroller whose writes clamp like the DOM. */
function scroller(scrollHeight = 2000, clientHeight = 500): Scroller & { h: number } {
  let top = 0;
  const el = {
    h: scrollHeight,
    get scrollTop() {
      return top;
    },
    set scrollTop(v: number) {
      top = Math.max(0, Math.min(v, el.h - clientHeight));
    },
    get scrollHeight() {
      return el.h;
    },
    clientHeight,
  };
  return el;
}

const make = (now = { t: 0 }) => createRoot(() => createBottomLock(() => now.t));

describe('bottom lock ([chat-load] rules)', () => {
  it('starts locked; jump pins to the bottom and clears the dot', () => {
    const lock = make();
    const el = scroller();
    expect(lock.locked()).toBe(true);
    lock.jump(el);
    expect(el.scrollTop).toBe(1500);
    expect(distanceToBottom(el)).toBe(0);
  });

  it('any reader gesture up detaches at once, before any scroll event', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    lock.gestureUp(el);
    expect(lock.locked()).toBe(false);
  });

  it('a reader scroll that moved up detaches, even 10 px (no 48 px dead zone)', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    el.scrollTop = 1490; // not our write
    expect(lock.onScroll(el)).toBe(true);
    expect(lock.locked()).toBe(false);
  });

  it('our own writes are never the reader: pinning / anchor writes keep the state', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    el.h = 2600;
    lock.grew(el);
    expect(lock.onScroll(el)).toBe(false);
    expect(lock.locked()).toBe(true);
    lock.detach();
    lock.set(el, 200);
    expect(lock.onScroll(el)).toBe(false);
    expect(lock.locked()).toBe(false);
  });

  it('nothing re-attaches but the reader reaching the bottom (scrolling down) or the jump', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    lock.gestureUp(el);
    el.scrollTop = 900;
    lock.onScroll(el);
    // Layout: content shrinks / grows, our anchor writes — still detached.
    lock.set(el, 950);
    lock.onScroll(el);
    el.h = 1900;
    lock.grew(el);
    expect(lock.locked()).toBe(false);
    // The reader scrolls down to within BOTTOM_PX.
    el.scrollTop = el.h - el.clientHeight - (BOTTOM_PX - 2);
    lock.onScroll(el);
    expect(lock.locked()).toBe(true);
  });

  it('a clamp after content shrank (scrollTop pulled to the new bottom) is not a detach', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    el.h = 1200; // clamps 1500 → 700
    el.scrollTop = el.scrollTop;
    expect(lock.onScroll(el)).toBe(true);
    expect(lock.locked()).toBe(true);
  });

  it('grew() pins while locked and keeps position while detached; geometry never flags unread', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    el.h = 2400;
    lock.grew(el);
    expect(el.scrollTop).toBe(1900);
    lock.detach();
    el.h = 3000;
    lock.grew(el);
    expect(el.scrollTop).toBe(1900);
    expect(lock.unread()).toBe(false);
    lock.arrived();
    expect(lock.unread()).toBe(true);
    lock.jump(el);
    expect(lock.unread()).toBe(false);
  });

  it('a view that cannot scroll is locked and shows from the top (F-12)', () => {
    const lock = make();
    const el = scroller(400, 500);
    lock.detach();
    lock.reconcile(el);
    expect(lock.locked()).toBe(true);
    expect(el.scrollTop).toBe(0);
  });

  it('scrolling() is true right after a reader scroll (flick / inertia) and false once idle', () => {
    const now = { t: 1000 };
    const lock = make(now);
    const el = scroller();
    lock.jump(el);
    el.scrollTop = 1000;
    lock.onScroll(el);
    expect(lock.scrolling()).toBe(true);
    now.t += SCROLL_IDLE_MS + 1;
    expect(lock.scrolling()).toBe(false);
  });

  it('owner-fix-9: the top mode is not detached for the dot, grew() does not pin, a reader scroll up releases it', () => {
    const lock = make();
    const el = scroller();
    lock.jump(el);
    lock.enterTop();
    expect(lock.mode()).toBe('top');
    lock.set(el, 900); // the sent message placed at the top
    expect(lock.onScroll(el)).toBe(false);
    el.h = 2400;
    lock.grew(el);
    expect(el.scrollTop).toBe(900);
    lock.arrived();
    expect(lock.unread()).toBe(false);
    el.scrollTop = 850; // the reader
    lock.onScroll(el);
    expect(lock.mode()).toBe('detached');
    lock.releaseTop();
    expect(lock.mode()).toBe('detached');
  });
});
