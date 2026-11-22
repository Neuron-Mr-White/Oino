/**
 * Arrow drag pad shared by the terminal toolbar and the floating
 * composer, so both feel the same. It behaves like a held arrow key on
 * a hardware keyboard: drag off-centre → one arrow (with a haptic
 * tick), keep holding → auto-repeat after a short delay at a steady
 * pace; the direction follows the finger; release or return to the
 * centre stops. A tap alone sends nothing.
 */
import { createSignal, onCleanup } from 'solid-js';
import { Move } from 'lucide-solid';
import { ArrowRepeat, type ArrowDirection } from '../lib/termToolbar';
import { hapticTick } from '../lib/haptics';

export type ArrowPadProps = {
  /** Sends one arrow in this direction. */
  onArrow: (direction: ArrowDirection) => void;
  haptics: boolean;
  label: string;
  /** Visual class (e.g. `term-key` or `composer-icon`). */
  class: string;
  iconSize: number;
};

export default function ArrowPad(props: ArrowPadProps) {
  const pad = new ArrowRepeat();
  const [dir, setDir] = createSignal<ArrowDirection | undefined>(undefined);
  const [active, setActive] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stopTimer = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  const schedule = () => {
    stopTimer();
    const delay = pad.nextDelay();
    if (delay === undefined) return;
    timer = setTimeout(() => {
      const direction = pad.repeat();
      if (!direction) return;
      props.onArrow(direction);
      schedule();
    }, delay);
  };
  const end = () => {
    stopTimer();
    pad.up();
    setDir(undefined);
    setActive(false);
  };
  onCleanup(end);
  return (
    <div
      role="slider"
      aria-label={props.label}
      aria-valuetext={dir() ?? 'idle'}
      tabindex={-1}
      class={`${props.class} term-arrow-pad`}
      data-active={active() ? '' : undefined}
      data-dir={dir()}
      // Keep focus where it is (the IME textarea / composer field).
      onMouseDown={(event) => event.preventDefault()}
      onContextMenu={(event) => event.preventDefault()}
      onPointerDown={(event) => {
        (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
        stopTimer();
        pad.down(event.clientX, event.clientY);
        setActive(true);
      }}
      onPointerMove={(event) => {
        if (!active()) return;
        const engaged = pad.move(event.clientX, event.clientY);
        setDir(pad.direction);
        if (engaged) {
          props.onArrow(engaged);
          void hapticTick(props.haptics, 'tick');
          schedule();
        } else if (!pad.direction) {
          stopTimer();
        }
      }}
      onPointerUp={end}
      onPointerCancel={end}
    >
      <Move size={props.iconSize} class="text-text" />
    </div>
  );
}
