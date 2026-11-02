// @vitest-environment jsdom
import { createRoot, createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createSmoothText,
  noteArrival,
  revealCps,
  revealState,
  setStreamSmoothing,
  SMOOTH_DEFAULTS,
  stepReveal,
  streamSmoothingEnabled,
} from './smoothStream';

/** Runs frames of `dt` ms until `shown` reaches the text or `maxMs` passes; returns the elapsed ms. */
function drain(s: ReturnType<typeof revealState>, text: string, t0: number, dt = 16, maxMs = 5000): number {
  let t = t0;
  while (s.shown < text.length && t - t0 < maxMs) {
    t += dt;
    stepReveal(s, text, t, dt);
  }
  return t - t0;
}

describe('stepReveal (pure)', () => {
  it('reveals gradually, never more than the backlog, carrying fractional debt', () => {
    const s = revealState();
    const text = 'x'.repeat(10);
    noteArrival(s, text.length, 0);
    const first = stepReveal(s, text, 16, 16);
    expect(first).toBeGreaterThan(0);
    expect(first).toBeLessThan(text.length);
    expect(drain(s, text, 16)).toBeLessThan(SMOOTH_DEFAULTS.maxLagMs);
    expect(s.shown).toBe(10);
    expect(s.debt).toBe(0);
  });

  it('speeds up with the backlog (catch-up) so a big burst drains within the lag ceiling', () => {
    expect(revealCps(2000)).toBeGreaterThan(revealCps(200));
    expect(revealCps(200)).toBeGreaterThan(revealCps(10));
    for (const n of [50, 400, 3000, 20000]) {
      const s = revealState();
      const text = 'y'.repeat(n);
      noteArrival(s, n, 0);
      expect(drain(s, text, 0)).toBeLessThanOrEqual(SMOOTH_DEFAULTS.maxLagMs + 16);
    }
  });

  it('a steady stream stays within the lag ceiling behind arrival', () => {
    const s = revealState();
    let text = '';
    let maxLag = 0;
    const arrivedAt: number[] = [];
    for (let t = 0; t < 3000; t += 16) {
      if (t % 48 === 0) {
        text += 'abcdefghijklmnopqrstuvwxyz '.repeat(2); // ~1100 chars/s, bursty
        noteArrival(s, text.length, t);
        arrivedAt.push(t);
      }
      stepReveal(s, text, t, 16);
      // Lag = age of the oldest arrival not fully shown yet.
      const marks = s.arrivals;
      if (marks.length) maxLag = Math.max(maxLag, t - marks[0]!.at);
    }
    expect(maxLag).toBeLessThanOrEqual(SMOOTH_DEFAULTS.maxLagMs);
  });

  it('a stalled frame is clamped (no dump), but the hard deadline still applies', () => {
    const s = revealState();
    const text = 'z'.repeat(1000);
    noteArrival(s, text.length, 0);
    stepReveal(s, text, 100, 100);
    expect(s.shown).toBeLessThan(1000);
    stepReveal(s, text, SMOOTH_DEFAULTS.maxLagMs, 16);
    expect(s.shown).toBe(1000);
  });

  it('never splits a surrogate pair', () => {
    const text = '😀'.repeat(50);
    const s = revealState();
    noteArrival(s, text.length, 0);
    for (let t = 16; s.shown < text.length; t += 16) {
      stepReveal(s, text, t, 16);
      expect(s.shown % 2).toBe(0);
    }
  });
});

describe('createSmoothText', () => {
  let frames: Array<FrameRequestCallback> = [];
  let clock = 0;
  const tick = (ms = 16) => {
    clock += ms;
    const run = frames;
    frames = [];
    run.forEach((f) => f(clock));
  };
  beforeEach(() => {
    frames = [];
    clock = 0;
    vi.stubGlobal('requestAnimationFrame', (f: FrameRequestCallback) => (frames.push(f), frames.length));
    vi.stubGlobal('cancelAnimationFrame', () => {
      frames = [];
    });
    vi.spyOn(performance, 'now').mockImplementation(() => clock);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setStreamSmoothing(undefined);
  });

  const mount = (initial: string, live = true) =>
    createRoot((dispose) => {
      const [text, setText] = createSignal(initial);
      const [isLive, setLive] = createSignal(live);
      const shown = createSmoothText(text, isLive);
      return { shown, setText, setLive, dispose };
    });

  it('animates a live stream and flushes instantly when the message ends', () => {
    setStreamSmoothing(true);
    const m = mount('');
    m.setText('Hello there, this is a streaming answer.');
    expect(m.shown()).toBe('');
    tick();
    const partial = m.shown();
    expect(partial.length).toBeGreaterThan(0);
    expect(partial.length).toBeLessThan(40);
    expect('Hello there, this is a streaming answer.'.startsWith(partial)).toBe(true);
    m.setText('Hello there, this is a streaming answer. More text arrives.');
    m.setLive(false); // message_end / stop / error
    expect(m.shown()).toBe('Hello there, this is a streaming answer. More text arrives.');
    m.dispose();
  });

  it('catches up within the lag ceiling', () => {
    setStreamSmoothing(true);
    const m = mount('');
    const big = 'w'.repeat(5000);
    m.setText(big);
    for (let i = 0; i < Math.ceil(SMOOTH_DEFAULTS.maxLagMs / 16) + 1; i++) tick();
    expect(m.shown()).toBe(big);
    m.dispose();
  });

  it('settled (not live) text and a long mid-stream remount show at once', () => {
    setStreamSmoothing(true);
    const settled = mount('Done answer', false);
    expect(settled.shown()).toBe('Done answer');
    settled.dispose();
    const long = 'p'.repeat(2000);
    const remount = mount(long, true);
    expect(remount.shown()).toBe(long);
    remount.dispose();
  });

  it('a non-extending change (edit / reset) snaps to the new text', () => {
    setStreamSmoothing(true);
    const m = mount('');
    m.setText('first version of the text');
    tick();
    m.setText('totally different');
    expect(m.shown()).toBe('totally different');
    m.dispose();
  });

  it('flushes when the tab is hidden', () => {
    setStreamSmoothing(true);
    const m = mount('');
    m.setText('x'.repeat(300));
    tick();
    expect(m.shown().length).toBeLessThan(300);
    const vis = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    document.dispatchEvent(new Event('visibilitychange'));
    expect(m.shown().length).toBe(300);
    vis.mockRestore();
    m.dispose();
  });

  it('prefers-reduced-motion: no animation, text passes straight through', () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('reduce'), addEventListener() {}, removeEventListener() {} }));
    expect(streamSmoothingEnabled()).toBe(false);
    const m = mount('');
    m.setText('instant text');
    expect(m.shown()).toBe('instant text');
    expect(frames.length).toBe(0);
    m.dispose();
  });

  it('honours the unipi.smoothStream=off switch', () => {
    localStorage.setItem('unipi.smoothStream', 'off');
    expect(streamSmoothingEnabled()).toBe(false);
    localStorage.removeItem('unipi.smoothStream');
    expect(streamSmoothingEnabled()).toBe(true);
  });
});
