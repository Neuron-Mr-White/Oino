/**
 * Toast stack tests (WP-A19 acceptance): info/success 3 s, warning 5 s,
 * error 8 s; an action buys 4 s more; identical toasts fold ×N; at most 3
 * visible; hover pauses and resumes with the remaining time.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createToastStack, toastTtl, TOAST_ERROR_MS, TOAST_EXIT_MS, TOAST_INFO_MS, TOAST_WARNING_MS, type ToastTimers } from './toasts';

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
  it('info 3 s, success 3 s, warning 5 s, error 8 s; +4 s with an action', () => {
    expect(toastTtl('info')).toBe(3000);
    expect(toastTtl('success')).toBe(3000);
    expect(toastTtl('warning')).toBe(5000);
    expect(toastTtl('error')).toBe(8000);
    expect(toastTtl('info', true)).toBe(7000);
    expect(toastTtl('warning', true)).toBe(9000);
    expect(toastTtl('error', true)).toBe(12_000);
  });
});

describe('timings', () => {
  it('an info toast auto-dismisses after 3 s (through the exit fade)', () => {
    const s = stack();
    s.push('Plan mode on');
    const ttl = due(TOAST_INFO_MS)!;
    expect(ttl).toBeTruthy();
    run(ttl);
    expect(s.list()).toHaveLength(1);
    expect(s.list()[0]!.leaving).toBe(true);
    run(due(TOAST_EXIT_MS)!);
    expect(s.list()).toHaveLength(0);
  });

  it('warnings and errors stay longer before auto-dismissing', () => {
    const s = stack();
    s.push('Flaky', 'warning');
    expect(due(TOAST_WARNING_MS)).toBeTruthy();
    s.push('Disk full', 'error');
    expect(due(TOAST_ERROR_MS)).toBeTruthy();
  });

  it('an action extends the lifetime (info + Undo = 7 s)', () => {
    const s = stack();
    s.push('Task archived', 'info', { label: 'Undo', run: () => {} });
    expect(due(7000)).toBeTruthy();
  });
});

describe('stack rules', () => {
  it('identical toasts (no action) fold into ×N with a fresh timer', () => {
    const s = stack();
    s.push('Copied');
    now += 2000;
    s.push('Copied');
    expect(s.list()).toHaveLength(1);
    expect(s.list()[0]!.count).toBe(2);
    expect(due(TOAST_INFO_MS)).toBeTruthy();
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
    expect(fired.filter((f) => f.ms === TOAST_EXIT_MS)).toHaveLength(0);
  });

  it('hover pauses and resumes with the remaining time', () => {
    const s = stack();
    const id = s.push('Copied');
    now += 2000; // 2 s in of 3 s
    s.pause(id);
    expect(due(1000)).toBeUndefined();
    expect(due(TOAST_INFO_MS)).toBeUndefined();
    now += 60_000;
    s.resume(id);
    expect(due(1000)).toBeTruthy();
    run(due(1000)!);
    expect(s.list()[0]!.leaving).toBe(true);
  });

  it('dismiss cancels the lifetime timer and starts exit fade', () => {
    const s = stack();
    const warn = s.push('Flaky', 'warning');
    s.dismiss(warn);
    expect(fired.some((f) => f.ms === TOAST_WARNING_MS)).toBe(false);
    expect(s.list()[0]!.leaving).toBe(true);
  });
});
