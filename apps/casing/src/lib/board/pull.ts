/**
 * Pull-to-refresh for a vertical scroller (UNI-130). Engages only when
 * the scroller is at the top and the gesture is mostly vertical (lane
 * pages also swipe sideways), reports a damped pull distance while
 * dragging and fires once on release past the threshold.
 */
export const PULL_THRESHOLD = 64;

export function attachPullToRefresh(
  el: HTMLElement,
  handlers: { onPull: (distance: number) => void; onRefresh: () => void; onArmed?: () => void },
): () => void {
  let startX = 0;
  let startY = 0;
  let mode: 'idle' | 'maybe' | 'pull' | 'off' = 'idle';
  let distance = 0;
  let armed = false;

  const start = (event: TouchEvent) => {
    if (event.touches.length !== 1 || el.scrollTop > 0) {
      mode = 'off';
      return;
    }
    startX = event.touches[0]!.clientX;
    startY = event.touches[0]!.clientY;
    mode = 'maybe';
    distance = 0;
    armed = false;
  };
  const move = (event: TouchEvent) => {
    if (mode === 'off' || mode === 'idle') return;
    const dx = event.touches[0]!.clientX - startX;
    const dy = event.touches[0]!.clientY - startY;
    if (mode === 'maybe') {
      if (Math.abs(dx) < 8 && Math.abs(dy) < 8) return;
      if (dy <= 0 || Math.abs(dx) > Math.abs(dy) || el.scrollTop > 0) {
        mode = 'off';
        return;
      }
      mode = 'pull';
    }
    if (event.cancelable) event.preventDefault();
    distance = Math.min(120, Math.max(0, dy) * 0.5);
    handlers.onPull(distance);
    if (!armed && distance >= PULL_THRESHOLD) {
      armed = true;
      handlers.onArmed?.();
    } else if (armed && distance < PULL_THRESHOLD) armed = false;
  };
  const end = () => {
    if (mode === 'pull') {
      handlers.onPull(0);
      if (distance >= PULL_THRESHOLD) handlers.onRefresh();
    }
    mode = 'idle';
    distance = 0;
  };
  el.addEventListener('touchstart', start, { passive: true });
  el.addEventListener('touchmove', move, { passive: false });
  el.addEventListener('touchend', end);
  el.addEventListener('touchcancel', end);
  return () => {
    el.removeEventListener('touchstart', start);
    el.removeEventListener('touchmove', move);
    el.removeEventListener('touchend', end);
    el.removeEventListener('touchcancel', end);
  };
}
