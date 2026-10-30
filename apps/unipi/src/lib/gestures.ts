/**
 * Terminal gesture classification (DESIGN.md §3.3): double-tap paste,
 * pinch font size, two-finger swipe pane switching, edge-swipe
 * tab/window switching. Pure mapping; the component owns pointers.
 */

/** A pointer pair during a two-finger gesture. */
export type TwoFingerState = {
  startX: number;
  currentX: number;
};

/** Swipe distance that triggers a pane/window switch. */
export const SWIPE_THRESHOLD_PX = 60;

/** Edge strip width for edge swipes. */
export const EDGE_PX = 24;

/** Font-size clamp for pinch (DESIGN.md §3.9). */
export const FONT_MIN = 8;
export const FONT_MAX = 32;

/** Two-finger horizontal swipe: which pane to focus next. */
export function twoFingerSwipe(state: TwoFingerState): 'pane_next' | 'pane_prev' | undefined {
  const delta = state.currentX - state.startX;
  if (Math.abs(delta) < SWIPE_THRESHOLD_PX) return undefined;
  // Swiping right moves focus to the pane on the left (content
  // follows the finger).
  return delta > 0 ? 'pane_prev' : 'pane_next';
}

/** Whether a single-finger drag started in a screen edge. */
export function edgeStart(startX: number, viewportWidth: number): 'left' | 'right' | undefined {
  if (startX <= EDGE_PX) return 'left';
  if (startX >= viewportWidth - EDGE_PX) return 'right';
  return undefined;
}

/** Edge swipe → next/previous herdr tab or tmux window. */
export function edgeSwipeDirection(
  edge: 'left' | 'right',
  deltaX: number,
): 'tab_next' | 'tab_prev' | undefined {
  if (Math.abs(deltaX) < SWIPE_THRESHOLD_PX) return undefined;
  if (edge === 'left') return deltaX < 0 ? 'tab_next' : 'tab_prev';
  return deltaX > 0 ? 'tab_next' : 'tab_prev';
}

/** Pinch scale → next font size, clamped to 8–32. */
export function pinchFontSize(current: number, scale: number): number {
  const next = Math.round(current * scale);
  return Math.min(FONT_MAX, Math.max(FONT_MIN, next));
}

/** Double-tap detection from a tap timestamp history (ms). */
export function isDoubleTap(taps: number[], windowMs = 300): boolean {
  if (taps.length < 2) return false;
  return taps[taps.length - 1] - taps[taps.length - 2] <= windowMs;
}

/** Long-press duration before selection mode arms. */
export const LONG_PRESS_MS = 600;

/** Identical bytes inside this window are the Android IME echo. */
export const IME_ECHO_WINDOW_MS = 20;

/**
 * Android IMEs (GBoard/HyperOS) double-commit some keystrokes: the
 * composition result and the key event both reach the terminal within
 * a few milliseconds with identical bytes. A held key repeats at
 * >=30ms, so a tight identical-payload window only eats the echo.
 */
export function isImeEcho(data: string, lastData: string, sinceLastMs: number): boolean {
  return data.length > 0 && data === lastData && sinceLastMs <= IME_ECHO_WINDOW_MS;
}
