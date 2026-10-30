/**
 * Knob drag-down decision (DESIGN §3.3, Moshi): the terminal screen
 * follows the finger; on release it slides away (→ background + Home)
 * when dragged past DISMISS_FRACTION of its height or flung downward
 * faster than DISMISS_VELOCITY, otherwise it snaps back. A fling back
 * up always snaps back; a cancelled pointer never dismisses.
 */
export const DISMISS_FRACTION = 0.22;
/** px/ms, downward positive. */
export const DISMISS_VELOCITY = 0.5;
/** A fling needs at least this much travel (px) to count. */
export const FLING_MIN_PX = 24;

export function knobRelease(input: {
  offset: number;
  height: number;
  velocity: number;
  cancelled?: boolean;
}): 'dismiss' | 'snap-back' {
  if (input.cancelled) return 'snap-back';
  if (input.velocity < -0.2) return 'snap-back';
  if (input.offset >= input.height * DISMISS_FRACTION) return 'dismiss';
  if (input.offset >= FLING_MIN_PX && input.velocity >= DISMISS_VELOCITY) return 'dismiss';
  return 'snap-back';
}
