/**
 * Rolling label (WP-A14, Z-92): model / thinking / mode names roll vertically
 * when they change (ZCode's `chat-model-select-menu` animation), 180 ms in and
 * out — inside DESIGN §1.4's 200 ms exit rule.
 *
 * It rolls only for a real change: the first paint, a change of `scope` (another chat opened
 * in the same chip) and the arrival of the real value after a placeholder (`ready` false →
 * true: "Model" → "Mock" while the chat loads), or its return to one, swap in place. Replaying the roll on every
 * chat open drew half-rolled glyphs ("Mo/Mock") over a value that never changed.
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

export function RollingLabel(props: { text: string; class?: string; testId?: string; scope?: string; ready?: boolean }) {
  const [leaving, setLeaving] = createSignal<string | undefined>();
  const [rolled, setRolled] = createSignal(false);
  let previous = props.text;
  let previousScope = props.scope;
  let previousReady = props.ready !== false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    const next = props.text;
    const scope = props.scope;
    const ready = props.ready !== false;
    // r8 P3-B: a value that goes BACK to its placeholder (a chat's runtime restarting: "Mock" → "Model") swaps in
    // place too — rolling it drew both words over each other for ~150 ms.
    const quiet = scope !== previousScope || !previousReady || !ready;
    previousScope = scope;
    previousReady = ready;
    if (next === previous) return;
    const prev = previous;
    previous = next;
    clearTimeout(timer);
    if (quiet) {
      setRolled(false);
      setLeaving(undefined);
      return;
    }
    setRolled(true);
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
        {(text) => <span class="roll-in" data-roll={rolled() ? '' : undefined}>{text}</span>}
      </Show>
    </span>
  );
}
