// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { BOTTOM_PX, createBottomLock, distanceToBottom } from './bottomLock';

/** A fake scroller: metrics the DOM element would report. */
function scroller(opts: { height: number; client: number; top?: number }) {
  return { scrollHeight: opts.height, clientHeight: opts.client, scrollTop: opts.top ?? 0 };
}

describe('bottom lock (Z-23/Z-24)', () => {
  it('starts locked; a user scroll up > 48 px detaches', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    lock.intent();
    el.scrollTop = 1000; // distance 600
    lock.onScroll(el);
    expect(lock.detached()).toBe(true);
  });

  it('stays locked for a small user scroll (≤ 48 px)', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    el.scrollTop = el.scrollHeight - el.clientHeight - 30;
    lock.intent();
    lock.onScroll(el);
    expect(lock.locked()).toBe(true);
  });

  it('layout-driven scrolls never detach (user intent beats layout)', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    el.scrollTop = 500; // virtualizer compensation, no intent
    lock.onScroll(el);
    expect(lock.locked()).toBe(true);
  });

  it('user intent expires: a scroll long after the input is layout', () => {
    vi.useFakeTimers();
    try {
      const lock = createBottomLock();
      const el = scroller({ height: 2000, client: 400 });
      lock.intent();
      vi.advanceTimersByTime(1000);
      el.scrollTop = 100;
      lock.onScroll(el);
      expect(lock.locked()).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it('a user scroll back within 48 px re-locks and clears unread', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    lock.intent();
    el.scrollTop = 800;
    lock.onScroll(el);
    expect(lock.detached()).toBe(true);
    lock.arrived(); // new content while detached → unread dot
    expect(lock.unread()).toBe(true);
    lock.intent();
    el.scrollTop = el.scrollHeight - el.clientHeight - 20;
    lock.onScroll(el);
    expect(lock.locked()).toBe(true);
    expect(lock.unread()).toBe(false);
  });

  it('grew() pins the bottom in the same frame while locked', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    el.scrollTop = 100;
    lock.grew(el);
    expect(el.scrollTop).toBe(el.scrollHeight - el.clientHeight);
  });

  it('grew() while detached keeps position and never flags unread (geometry is not news)', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    lock.detach();
    el.scrollTop = 700;
    lock.grew(el);
    expect(el.scrollTop).toBe(700);
    expect(lock.unread()).toBe(false);
  });

  it('arrived() flags unread only while detached', () => {
    const lock = createBottomLock();
    lock.arrived();
    expect(lock.unread()).toBe(false);
    lock.detach();
    lock.arrived();
    expect(lock.unread()).toBe(true);
  });

  it('jump() locks, pins instantly and clears the dot', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    lock.detach();
    lock.arrived();
    lock.jump(el);
    expect(lock.locked()).toBe(true);
    expect(lock.unread()).toBe(false);
    expect(el.scrollTop).toBe(el.scrollHeight - el.clientHeight);
  });

  it('a view that cannot overflow is always lockable', () => {
    const lock = createBottomLock();
    lock.detach();
    const el = scroller({ height: 300, client: 400 });
    lock.reconcile(el);
    expect(lock.locked()).toBe(true);
  });

  it('reaching the bottom re-engages even without fresh intent', () => {
    const lock = createBottomLock();
    const el = scroller({ height: 2000, client: 400 });
    lock.detach();
    lock.arrived();
    expect(lock.unread()).toBe(true);
    el.scrollTop = el.scrollHeight - el.clientHeight; // user landed on the bottom
    lock.onScroll(el);
    expect(lock.locked()).toBe(true);
    expect(lock.unread()).toBe(false);
  });

  it('distanceToBottom', () => {
    expect(distanceToBottom(scroller({ height: 1000, client: 400, top: 500 }))).toBe(100);
  });

  it('the threshold is ZCode’s 48 px', () => {
    expect(BOTTOM_PX).toBe(48);
  });
});
