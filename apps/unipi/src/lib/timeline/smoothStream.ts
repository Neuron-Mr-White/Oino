/**
 * Smooth reveal for the live streaming message (ui-stream).
 *
 * The host coalesces deltas per 16 ms and providers deliver tokens in uneven
 * bursts, so binding the DOM to arrival dumps whole phrases at once. This
 * display-side smoother buffers the arrived text and reveals it on
 * `requestAnimationFrame` at an adaptive rate:
 *
 *   cps = max(BASE + backlog^1.25 · PRESSURE,  backlog / TARGET_LAG)
 *
 * The first term is the backlog-pressure curve of dsh-smooth-stream's reveal
 * engine (Laplace-bit/dsh-smooth-stream, MIT — `computeAdaptiveQueueStep`:
 * 90 cps base, exponent 1.25, factor 0.85, fractional "debt" carried between
 * frames); the second keeps the steady-state lag near TARGET_LAG. A hard
 * deadline reveals anything that arrived more than MAX_LAG_MS ago, and a
 * frame-stall clamp keeps a long frame from dumping the buffer.
 *
 * Only the display is smoothed — stored state and the protocol carry the
 * full text. The reveal snaps to the full text when the message stops being
 * live (end / stop / error), when the tab is hidden, under
 * `prefers-reduced-motion: reduce`, or when the new text isn't an extension
 * of the old (edit / reset).
 */
import { createEffect, createSignal, getOwner, onCleanup, untrack, type Accessor } from 'solid-js';

export type SmoothConfig = {
  /** Reveal speed with an empty backlog (chars/s). */
  baseCps: number;
  pressureExp: number;
  pressureFactor: number;
  /** Steady-state lag the backlog term aims for (ms). */
  targetLagMs: number;
  /** Hard ceiling: text older than this is shown regardless of speed (ms). */
  maxLagMs: number;
  /** Frame delta clamp so a stalled frame doesn't dump the buffer (ms). */
  maxFrameMs: number;
};

export const SMOOTH_DEFAULTS: SmoothConfig = {
  baseCps: 90,
  pressureExp: 1.25,
  pressureFactor: 0.85,
  targetLagMs: 180,
  maxLagMs: 350,
  maxFrameMs: 50,
};

/** Pure reveal state (one live message). */
export type RevealState = {
  /** UTF-16 units shown. */
  shown: number;
  /** Fractional characters owed from earlier frames. */
  debt: number;
  /** Arrival marks: text up to `end` arrived at `at` (ms). Ascending. */
  arrivals: Array<{ end: number; at: number }>;
};

export const revealState = (shown = 0): RevealState => ({ shown, debt: 0, arrivals: [] });

/** Records that the target grew to `len` at `now`. */
export function noteArrival(s: RevealState, len: number, now: number): void {
  const last = s.arrivals[s.arrivals.length - 1];
  if (last && last.end >= len) return;
  s.arrivals.push({ end: len, at: now });
}

/** Reveal speed for a backlog (chars/s). */
export function revealCps(backlog: number, cfg: SmoothConfig = SMOOTH_DEFAULTS): number {
  if (backlog <= 0) return 0;
  return Math.max(cfg.baseCps + Math.pow(backlog, cfg.pressureExp) * cfg.pressureFactor, (backlog * 1000) / cfg.targetLagMs);
}

/** Don't cut a surrogate pair in half. */
function snapCodePoint(text: string, at: number): number {
  if (at <= 0 || at >= text.length) return at;
  const c = text.charCodeAt(at - 1);
  return c >= 0xd800 && c <= 0xdbff ? at + 1 : at;
}

/**
 * Advances `s` by one frame towards `text` (pure apart from mutating `s`).
 * Returns the new `shown` (UTF-16 units).
 */
export function stepReveal(s: RevealState, text: string, now: number, dtMs: number, cfg: SmoothConfig = SMOOTH_DEFAULTS): number {
  const len = text.length;
  if (s.shown >= len) {
    s.shown = len;
    s.debt = 0;
    s.arrivals.length = 0;
    return len;
  }
  const dt = Math.min(Math.max(dtMs, 0), cfg.maxFrameMs);
  const backlog = len - s.shown;
  const owed = s.debt + (revealCps(backlog, cfg) * dt) / 1000;
  const step = Math.min(backlog, Math.floor(owed));
  s.debt = owed - step;
  let shown = s.shown + step;
  // Hard deadline: everything that arrived ≥ maxLagMs ago is due now.
  let i = 0;
  while (i < s.arrivals.length && now - s.arrivals[i]!.at >= cfg.maxLagMs) {
    shown = Math.max(shown, s.arrivals[i]!.end);
    i++;
  }
  if (i) s.arrivals.splice(0, i);
  shown = snapCodePoint(text, Math.min(shown, len));
  if (shown >= len) {
    s.debt = 0;
    s.arrivals.length = 0;
  }
  s.shown = shown;
  return shown;
}

/* ── environment ──────────────────────────────────────────────────── */

let override: boolean | undefined;

/** Force smoothing on/off (tests, debugging); `undefined` = auto. */
export function setStreamSmoothing(on: boolean | undefined): void {
  override = on;
}

const reducedMotion = (): boolean => {
  try {
    return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
};

/** Whether the live message should animate right now. */
export function streamSmoothingEnabled(): boolean {
  if (typeof requestAnimationFrame !== 'function') return false;
  if (override !== undefined) return override;
  if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return false;
  try {
    if (localStorage.getItem('unipi.smoothStream') === 'off') return false;
  } catch {
    /* no storage */
  }
  return !reducedMotion();
}

/** Short initial text animates from empty (a fresh stream); longer text (a remount mid-stream) shows at once. */
const FRESH_MAX = 160;

export type SmoothOptions = {
  /** Minimum gap between commits (ms) — the caller's render-cost throttle. */
  minFrameMs?: () => number;
  config?: SmoothConfig;
};

/**
 * Reactive smoothed view of `text` while `live()`; passthrough otherwise.
 * Call inside a component/root.
 */
export function createSmoothText(text: Accessor<string>, live: Accessor<boolean>, opts: SmoothOptions = {}): Accessor<string> {
  const cfg = opts.config ?? SMOOTH_DEFAULTS;
  const initial = untrack(text);
  const animateFromStart = untrack(live) && initial.length <= FRESH_MAX && streamSmoothingEnabled();
  const st = revealState(animateFromStart ? 0 : initial.length);
  let target = initial;
  const [shown, setShown] = createSignal(animateFromStart ? '' : initial);
  let raf = 0;
  let lastFrame = 0;
  let lastCommit = 0;
  const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
  if (animateFromStart) noteArrival(st, initial.length, now());

  const stop = () => {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };
  const flush = () => {
    stop();
    st.shown = target.length;
    st.debt = 0;
    st.arrivals.length = 0;
    setShown(target);
  };
  const frame = (t: number) => {
    raf = 0;
    if (!streamSmoothingEnabled()) return flush();
    const dt = lastFrame ? t - lastFrame : 16;
    lastFrame = t;
    const gap = opts.minFrameMs?.() ?? 0;
    if (gap > 0 && t - lastCommit < gap && st.shown < target.length) {
      // Throttled (expensive render): keep integrating, commit later.
      stepReveal(st, target, t, dt, cfg);
      raf = requestAnimationFrame(frame);
      return;
    }
    const n = stepReveal(st, target, t, dt, cfg);
    lastCommit = t;
    setShown(n >= target.length ? target : target.slice(0, n));
    if (n < target.length) raf = requestAnimationFrame(frame);
    else lastFrame = 0;
  };
  const kick = () => {
    if (!raf) raf = requestAnimationFrame(frame);
  };

  createEffect(() => {
    const next = text();
    const isLive = live();
    untrack(() => {
      const extends_ = next.length >= target.length && next.startsWith(target);
      target = next;
      if (!isLive || !extends_ || !streamSmoothingEnabled()) return flush();
      if (st.shown >= next.length) return setShown(next);
      noteArrival(st, next.length, now());
      kick();
    });
  });

  if (typeof document !== 'undefined') {
    const onVis = () => {
      if (document.visibilityState === 'hidden') flush();
    };
    document.addEventListener('visibilitychange', onVis);
    if (getOwner()) onCleanup(() => document.removeEventListener('visibilitychange', onVis));
  }
  if (getOwner()) onCleanup(stop);
  return shown;
}
