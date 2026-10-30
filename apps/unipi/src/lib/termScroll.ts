/**
 * One-finger touch scrolling for the terminal (DESIGN §3.3).
 *
 * Full-screen TUIs (pi, editors, pagers) turn on mouse reporting and
 * draw on the alternate screen, so xterm's own scrollback is empty and
 * native touch-panning does nothing useful.
 * A vertical drag is therefore translated into whatever the app on
 * the other end understands:
 *
 * - mouse tracking on  → wheel reports (SGR `\x1b[<64;col;rowM` up,
 *   `65` down; legacy X10 bytes when the app did not ask for SGR)
 * - alt screen, no mouse → Up/Down arrow keys (xterm's "alternate
 *   scroll" behaviour)
 * - normal screen       → scroll xterm's local scrollback
 *
 * Pure mapping only; the terminal pane (wave 3, WP-A31) owns the pointers.
 */

export type ScrollTarget = 'mouse' | 'alt-keys' | 'scrollback';
export type MouseEncoding = 'DEFAULT' | 'SGR' | 'SGR_PIXELS' | string;

/** Lines a TUI scrolls per wheel notch (the common default). */
export const LINES_PER_WHEEL = 3;

/** Drag distance before a one-finger touch counts as a scroll. */
export const SCROLL_SLOP_PX = 8;

/**
 * Content lines moved per finger travel of one cell height. 1.0 was a
 * strict 1:1 track (UNI-125) and felt sluggish next to native lists,
 * where a flick also carries; 1.6 matches the feel of an Android list
 * at terminal line heights (UNI-127).
 */
export const SCROLL_GAIN = 1.6;

/**
 * Finger px per emitted scroll step: one wheel notch scrolls
 * `LINES_PER_WHEEL` lines remotely, one arrow / scrollLines step scrolls
 * one line locally.
 */
export function scrollStepPx(
  target: ScrollTarget,
  cellHeight: number,
  gain = SCROLL_GAIN,
): number {
  const lines = target === 'mouse' ? LINES_PER_WHEEL : 1;
  return Math.max(1, (cellHeight * lines) / gain);
}

export function scrollTargetFor(
  mouseTrackingMode: string,
  bufferType: 'normal' | 'alternate',
): ScrollTarget {
  if (mouseTrackingMode !== 'none') return 'mouse';
  if (bufferType === 'alternate') return 'alt-keys';
  return 'scrollback';
}

/**
 * Wheel report for one notch at 1-based cell `col`/`row`. `up` means
 * "show earlier content" (finger dragged down).
 */
export function wheelSequence(
  direction: 'up' | 'down',
  col: number,
  row: number,
  encoding: MouseEncoding,
  pixel?: { x: number; y: number },
): string {
  const button = direction === 'up' ? 64 : 65;
  if (encoding === 'SGR') return `\x1b[<${button};${col};${row}M`;
  if (encoding === 'SGR_PIXELS') {
    const x = Math.max(1, Math.round(pixel?.x ?? col));
    const y = Math.max(1, Math.round(pixel?.y ?? row));
    return `\x1b[<${button};${x};${y}M`;
  }
  // X10/"DEFAULT": three bytes offset by 32, positions capped at 223.
  const clamp = (n: number) => String.fromCharCode(32 + Math.min(223, Math.max(1, n)));
  return `\x1b[M${String.fromCharCode(32 + button)}${clamp(col)}${clamp(row)}`;
}

/**
 * Left click (press + release) at 1-based cell `col`/`row`, for taps
 * while the app tracks the mouse (UNI-128: a TUI's header buttons).
 * SGR ends the press with `M` and the release with `m`; X10 reports a
 * release as button 3. `withRelease` is false for X10-only tracking
 * (mode 9 reports presses only).
 */
export function clickSequence(
  col: number,
  row: number,
  encoding: MouseEncoding,
  pixel?: { x: number; y: number },
  withRelease = true,
): string {
  if (encoding === 'SGR' || encoding === 'SGR_PIXELS') {
    const pixels = encoding === 'SGR_PIXELS';
    const x = pixels ? Math.max(1, Math.round(pixel?.x ?? col)) : col;
    const y = pixels ? Math.max(1, Math.round(pixel?.y ?? row)) : row;
    return `\x1b[<0;${x};${y}M` + (withRelease ? `\x1b[<0;${x};${y}m` : '');
  }
  const clamp = (n: number) => String.fromCharCode(32 + Math.min(223, Math.max(1, n)));
  const at = `${clamp(col)}${clamp(row)}`;
  return `\x1b[M${String.fromCharCode(32)}${at}` + (withRelease ? `\x1b[M${String.fromCharCode(35)}${at}` : '');
}

/** Arrow key for alternate-screen scrolling. */
export function arrowSequence(direction: 'up' | 'down', applicationCursor: boolean): string {
  const final = direction === 'up' ? 'A' : 'B';
  return applicationCursor ? `\x1bO${final}` : `\x1b[${final}`;
}

/** Converts a client point to a 1-based terminal cell, clamped. */
export function cellAt(
  point: { x: number; y: number },
  origin: { left: number; top: number },
  cell: { width: number; height: number },
  size: { cols: number; rows: number },
): { col: number; row: number } {
  const col = Math.floor((point.x - origin.left) / cell.width) + 1;
  const row = Math.floor((point.y - origin.top) / cell.height) + 1;
  return {
    col: Math.min(size.cols, Math.max(1, col)),
    row: Math.min(size.rows, Math.max(1, row)),
  };
}

/**
 * Accumulates finger movement into whole scroll steps. `push(dy)`
 * (positive = finger moved down) returns the number of steps to emit:
 * positive = scroll up (earlier content), negative = scroll down.
 */
export class ScrollAccumulator {
  private remainder = 0;
  constructor(private stepPx: number) {}
  setStep(stepPx: number): void {
    this.stepPx = Math.max(1, stepPx);
  }
  push(dy: number): number {
    this.remainder += dy;
    const steps = Math.trunc(this.remainder / this.stepPx);
    this.remainder -= steps * this.stepPx;
    return steps;
  }
  reset(): void {
    this.remainder = 0;
  }
}

/** Velocity (px/ms) over recent samples, newest last. */
export function releaseVelocity(samples: { t: number; y: number }[], windowMs = 90): number {
  if (samples.length < 2) return 0;
  const last = samples[samples.length - 1];
  let first = samples[samples.length - 2];
  for (let i = samples.length - 2; i >= 0; i -= 1) {
    if (last.t - samples[i].t > windowMs) break;
    first = samples[i];
  }
  const dt = last.t - first.t;
  return dt > 0 ? (last.y - first.y) / dt : 0;
}

/** Momentum: friction per 16 ms frame and the cut-off speed (px/ms).
 * UNI-125 had 0.94 / 0.05 / 0.35 (a fling died in ~0.5 s and needed a
 * hard flick); UNI-127 glides about 1.5 s like a native list. */
export const MOMENTUM_FRICTION = 0.965;
export const MOMENTUM_MIN_VELOCITY = 0.03;
export const MOMENTUM_START_VELOCITY = 0.2;

/** Next velocity after `dtMs` of friction. */
export function decayVelocity(velocity: number, dtMs: number): number {
  return velocity * Math.pow(MOMENTUM_FRICTION, dtMs / 16);
}
