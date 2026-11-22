/**
 * Resize handle between shell frames (Z-4): a 4 px hit area on the seam with
 * a 2 px line on hover / focus / drag (ZCode affordance, unipi flush look).
 * Drag with the pointer, arrows (Shift = bigger steps), double-click resets.
 * `onSize` receives the new size computed from the drag start.
 */
import type { JSX } from 'solid-js';

export function ResizeHandle(props: {
  /** `x`: a vertical seam (drag left/right); `y`: a horizontal seam (drag up/down). */
  axis: 'x' | 'y';
  label: string;
  /** The current size of the frame being resized. */
  size: number;
  /** +1 when dragging right/down grows the frame, -1 when it shrinks it. */
  dir: 1 | -1;
  onSize: (size: number) => void;
  onReset?: () => void;
  min?: number;
  max?: number;
  class?: string;
  testId?: string;
  style?: JSX.CSSProperties;
}) {
  let start: { pos: number; size: number } | undefined;
  const pos = (e: PointerEvent) => (props.axis === 'x' ? e.clientX : e.clientY);
  const onDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    start = { pos: pos(e), size: props.size };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    document.documentElement.dataset.resizing = props.axis;
  };
  const onMove = (e: PointerEvent) => {
    if (!start) return;
    props.onSize(start.size + (pos(e) - start.pos) * props.dir);
  };
  const onUp = () => {
    start = undefined;
    delete document.documentElement.dataset.resizing;
  };
  const onKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 64 : 16;
    const grow = props.axis === 'x' ? 'ArrowRight' : 'ArrowDown';
    const shrink = props.axis === 'x' ? 'ArrowLeft' : 'ArrowUp';
    if (e.key === grow) props.onSize(props.size + step * props.dir);
    else if (e.key === shrink) props.onSize(props.size - step * props.dir);
    else return;
    e.preventDefault();
  };
  return (
    <div
      class={`resize-handle${props.class ? ` ${props.class}` : ''}`}
      data-axis={props.axis}
      role="separator"
      aria-orientation={props.axis === 'x' ? 'vertical' : 'horizontal'}
      aria-label={props.label}
      aria-valuenow={Math.round(props.size)}
      aria-valuemin={props.min}
      aria-valuemax={props.max}
      tabindex="0"
      style={props.style}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onDblClick={() => props.onReset?.()}
      onKeyDown={onKey}
      data-testid={props.testId}
    />
  );
}
