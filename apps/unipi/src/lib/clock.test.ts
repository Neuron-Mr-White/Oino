import { afterEach, describe, expect, it, vi } from 'vitest';
import { createEffect, createRoot, createSignal } from 'solid-js';
import { clockNow, clockStats, resetClocks, useClock } from './clock';

describe('shared clock', () => {
  afterEach(() => {
    resetClocks();
    vi.useRealTimers();
  });

  it('runs one timer for many readers and stops when the last one goes', () => {
    vi.useFakeTimers();
    const seen: number[] = [];
    const disposers: Array<() => void> = [];
    for (let i = 0; i < 50; i++) {
      createRoot((dispose) => {
        disposers.push(dispose);
        createEffect(() => seen.push(clockNow(1000)));
      });
    }
    expect(clockStats()).toMatchObject({ timers: 1, readers: 50 });
    vi.advanceTimersByTime(1000);
    expect(seen.length).toBe(100);
    for (const d of disposers) d();
    expect(clockStats()).toMatchObject({ timers: 0, readers: 0 });
  });

  it('useClock ticks only while active', () => {
    vi.useFakeTimers();
    const [live, setLive] = createSignal(true);
    let runs = 0;
    const dispose = createRoot((d) => {
      const now = useClock(live);
      createEffect(() => {
        now();
        runs++;
      });
      return d;
    });
    vi.advanceTimersByTime(2000);
    expect(runs).toBe(3);
    setLive(false);
    expect(clockStats().timers).toBe(0);
    vi.advanceTimersByTime(5000);
    expect(runs).toBe(4);
    dispose();
  });

  it('is a plain Date.now() outside a tracking scope', () => {
    expect(Math.abs(clockNow(1000) - Date.now())).toBeLessThan(50);
    expect(clockStats().timers).toBe(0);
  });
});
