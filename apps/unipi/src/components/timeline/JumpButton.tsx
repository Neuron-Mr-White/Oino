/**
 * Jump-to-bottom button (WP-A6, Z-23/Z-24): only when the lock is detached
 * AND the view overflows; a small dot marks content that arrived while
 * detached; the gradient mask behind it (`.tl-jump-fade`) is in
 * timeline.css.
 */
import { Show } from 'solid-js';
import { ArrowDown } from 'lucide-solid';

export function JumpButton(props: { show: boolean; unread: boolean; onJump: () => void }) {
  return (
    <Show when={props.show}>
      <button type="button" class="tl-jump jump-btn" aria-label="Jump to the latest" data-testid="tl-jump" onClick={props.onJump}>
        <ArrowDown size={18} />
        <Show when={props.unread}>
          <span class="tl-jump-dot" data-testid="tl-jump-dot" aria-hidden="true" />
        </Show>
      </button>
    </Show>
  );
}
