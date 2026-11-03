/**
 * Shared, low-rate clocks (perf-app): every "Working for 12s" / "agent 4m" /
 * countdown in the app reads one of these instead of owning a `setInterval`.
 *
 *   const elapsed = () => (running() ? clockNow(1000) - startedAt : durationMs);
 *
 * - One timer per period for the whole app (coalesced), not one per card.
 * - The timer runs only while some live computation reads the clock: a
 *   reader registers on its owner and unregisters on cleanup / re-run, so a
 *   settled card that stops reading it costs nothing.
 * - Paused while the document is hidden; the first visible frame ticks at once.
 * - Read outside a tracking scope it is just `Date.now()` (no subscription).
 */
import { createSignal, getListener, onCleanup, type Accessor } from 'solid-js';

type Clock = { now: Accessor<number>; set: (n: number) => void; readers: number; timer?: ReturnType<typeof setInterval> };

const clocks = new Map<number, Clock>();
let visibilityHooked = false;

const hidden = () => typeof document !== 'undefined' && document.hidden === true;

function start(period: number, c: Clock): void {
  if (c.timer !== undefined || c.readers <= 0 || hidden()) return;
  c.timer = setInterval(() => c.set(Date.now()), period);
}

function stop(c: Clock): void {
  if (c.timer === undefined) return;
  clearInterval(c.timer);
  c.timer = undefined;
}

function hookVisibility(): void {
  if (visibilityHooked || typeof document === 'undefined') return;
  visibilityHooked = true;
  document.addEventListener('visibilitychange', () => {
    for (const [period, c] of clocks) {
      if (hidden()) stop(c);
      else if (c.readers > 0) {
        c.set(Date.now());
        start(period, c);
      }
    }
  });
}

function clockOf(period: number): Clock {
  let c = clocks.get(period);
  if (!c) {
    const [now, set] = createSignal(Date.now(), { equals: false });
    c = { now, set, readers: 0 };
    clocks.set(period, c);
    hookVisibility();
  }
  return c;
}

/** Now, refreshed every `periodMs` while read from a live computation (see module doc). */
export function clockNow(periodMs = 1000): number {
  const c = clockOf(periodMs);
  if (getListener()) {
    c.readers++;
    start(periodMs, c);
    onCleanup(() => {
      c.readers = Math.max(0, c.readers - 1);
      if (c.readers === 0) stop(c);
    });
    // A long-idle clock's last value may be stale: never report a time behind the wall clock by a period.
    const v = c.now();
    return Date.now() - v > periodMs ? Date.now() : v;
  }
  return Date.now();
}

/** `clockNow` as an accessor that ticks only while `active()` (else a still `Date.now()` per read). */
export function useClock(active: () => boolean = () => true, periodMs = 1000): Accessor<number> {
  return () => (active() ? clockNow(periodMs) : Date.now());
}

/** Idle diagnostics (perf harness `window.__UNIPI_IDLE__`): running clock timers and readers. */
export function clockStats(): { timers: number; readers: number; periods: number[] } {
  let timers = 0;
  let readers = 0;
  const periods: number[] = [];
  for (const [p, c] of clocks) {
    readers += c.readers;
    if (c.timer !== undefined) {
      timers++;
      periods.push(p);
    }
  }
  return { timers, readers, periods };
}

/** Tests: drop every clock (and its timer). */
export function resetClocks(): void {
  for (const c of clocks.values()) stop(c);
  clocks.clear();
}
