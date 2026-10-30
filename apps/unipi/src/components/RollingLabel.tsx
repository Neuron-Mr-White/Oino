/**
 * Rolling label (WP-A14, Z-92): model / thinking / mode names roll vertically
 * when they change (ZCode's `chat-model-select-menu` animation), 180 ms in and
 * out — inside DESIGN §1.4's 200 ms exit rule.
 *
 * Reduced motion: with `prefers-reduced-motion: reduce` (or no `matchMedia`,
 * as in jsdom) the text swaps instantly and no leaving layer is rendered; the
 * stylesheet also drops both keyframes, so a user toggling the preference
 * mid-roll cannot be left with a stranded half-rolled label.
 */
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import './rolling.css';

/** One roll's length (in and out run together). */
export const ROLL_MS = 180;

/** Does the user ask for reduced motion? (`false` where `matchMedia` does not exist.) */
export function prefersReducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches === true;
}

export function RollingLabel(props: { text: string; class?: string; testId?: string }) {
  const [leaving, setLeaving] = createSignal<string | undefined>();
  let previous = props.text;
  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    const next = props.text;
    if (next === previous) return;
    const prev = previous;
    previous = next;
    clearTimeout(timer);
    if (prefersReducedMotion()) {
      setLeaving(undefined);
      return;
    }
    setLeaving(prev);
    timer = setTimeout(() => setLeaving(undefined), ROLL_MS);
  });
  onCleanup(() => clearTimeout(timer));

  return (
    <span
      class={`roll-label${props.class ? ` ${props.class}` : ''}`}
      data-testid={props.testId}
      data-reduced={prefersReducedMotion() ? '' : undefined}
    >
      <Show when={leaving()}>
        {(prev) => (
          <span class="roll-out" aria-hidden="true">
            {prev()}
          </span>
        )}
      </Show>
      {/* keyed: a new text remounts the layer, which restarts the roll */}
      <Show when={props.text} keyed>
        {(text) => <span class="roll-in">{text}</span>}
      </Show>
    </span>
  );
}
