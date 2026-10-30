/**
 * Toast stack tests (WP-A19 acceptance): info 4 s, warning 6 s, error sticky;
 * an action buys 4 s more; identical toasts fold ×N; at most 3 visible;
 * hover pauses and resumes with the remaining time.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createToastStack, toastTtl, TOAST_EXIT_MS, type ToastTimers } from './toasts';

type Fired = { ms: number; cb: () => void };
let fired: Fired[];
let now: number;
const timers = (): ToastTimers => ({
  set: (ms, cb) => {
    const id = { ms, cb } as Fired;
    fired.push(id);
    return id;
  },
  clear: (h) => {
    const i = fired.indexOf(h as Fired);
    if (i >= 0) fired.splice(i, 1);
  },
});
const stack = () => createToastStack(timers(), () => now);
/** Runs the earliest due timer (fake clock). */
const run = (id: Fired) => {
  now += id.ms;
  fired = fired.filter((f) => f !== id);
  id.cb();
};
const due = (ms: number) => fired.find((f) => f.ms === ms);

beforeEach(() => {
  fired = [];
  now = 10_000;
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe('toastTtl', () => {
  it('info 4 s, success 4 s, warning 6 s, error sticky; +4 s with an action', () => {
    expect(toastTtl('info')).toBe(4000);
    expect(toastTtl('success')).toBe(4000);
    expect(toastTtl('warning')).toBe(6000);
    expect(toastTtl('error')).toBe(0);
    expect(toastTtl('info', true)).toBe(8000);
    expect(toastTtl('warning', true)).toBe(10_000);
    expect(toastTtl('error', true)).toBe(0);
  });
});

describe('timings', () => {
  it('an info toast dismisses after 4 s (through the 200 ms exit fade)', () => {
    const s = stack();
    s.push('Copied');
    const ttl = due(4000)!;
    run(ttl);
    expect(s.list()).toHaveLength(1); // leaving
    expect(s.list()[0]!.leaving).toBe(true);
    run(due(TOAST_EXIT_MS)!);
    expect(s.list()).toHaveLength(0);
  });

  it('a warning goes after 6 s', () => {
    const s = stack();
    s.push('Flaky', 'warning');
    expect(due(6000)).toBeTruthy();
  });

  it('an error is sticky: no lifetime timer, only the manual one', () => {
    const s = stack();
    s.push('Disk full', 'error');
    expect(due(4000)).toBeUndefined();
    expect(due(6000)).toBeUndefined();
    expect(due(0)).toBeUndefined();
    expect(s.list()[0]!.leaving).toBeUndefined();
  });

  it('an action extends the lifetime (info + Undo = 8 s)', () => {
    const s = stack();
    s.push('Task archived', 'info', { label: 'Undo', run: () => {} });
    expect(due(8000)).toBeTruthy();
  });
});

describe('stack rules', () => {
  it('identical toasts (no action) fold into ×N with a fresh timer', () => {
    const s = stack();
    s.push('Copied');
    now += 2000; // 2 s in
    s.push('Copied');
    expect(s.list()).toHaveLength(1);
    expect(s.list()[0]!.count).toBe(2);
    expect(due(4000)).toBeTruthy(); // a fresh 4 s, not the remaining 2 s
  });

  it('an action toast never folds', () => {
    const s = stack();
    s.push('Moved', 'info', { label: 'Undo', run: () => {} });
    s.push('Moved', 'info', { label: 'Undo', run: () => {} });
    expect(s.list()).toHaveLength(2);
  });

  it('at most 3 visible: the oldest drops at once', () => {
    const s = stack();
    s.push('one', 'error');
    s.push('two', 'error');
    s.push('three', 'error');
    s.push('four', 'error');
    expect(s.list().map((t) => t.message)).toEqual(['two', 'three', 'four']);
    // 'one' is gone outright — no exit fade timer left for it.
    expect(fired.filter((f) => f.ms === TOAST_EXIT_MS)).toHaveLength(0);
  });

  it('hover pauses and resumes with the remaining time', () => {
    const s = stack();
    const id = s.push('Copied');
    now += 3000; // 3 s in
    s.pause(id);
    expect(due(1000)).toBeUndefined(); // timer cleared
    expect(due(4000)).toBeUndefined();
    now += 60_000; // a long hover
    s.resume(id);
    expect(due(1000)).toBeTruthy(); // exactly the remaining second
    run(due(1000)!);
    expect(s.list()[0]!.leaving).toBe(true);
  });

  it('pause on a sticky toast is a no-op; dismiss cancels the timer', () => {
    const s = stack();
    const id = s.push('Disk full', 'error');
    s.pause(id);
    s.resume(id);
    expect(s.list()[0]!.leaving).toBeUndefined();
    const warn = s.push('Flaky', 'warning');
    s.dismiss(warn);
    expect(fired.some((f) => f.ms === 6000)).toBe(false);
  });
});
