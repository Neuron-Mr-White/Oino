import { afterEach, describe, expect, it, vi } from 'vitest';
import { FIRST_PAINT_FALLBACK_MS, firstPaintDone, resetFirstPaint } from './firstPaint';

afterEach(() => {
  vi.useRealTimers();
  resetFirstPaint(true);
});

describe('firstPaintDone (w2-ui2 T1)', () => {
  it('is false at page load and turns true after a frame (or the fallback timer), once', () => {
    vi.useFakeTimers();
    resetFirstPaint(false);
    expect(firstPaintDone()).toBe(false);
    vi.advanceTimersByTime(FIRST_PAINT_FALLBACK_MS);
    expect(firstPaintDone()).toBe(true);
  });
});
