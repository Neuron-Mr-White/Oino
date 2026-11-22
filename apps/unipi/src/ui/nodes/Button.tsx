/**
 * `Button` (ui-protocol §4.5.2): `.btn .btn-*` (44 px pill); `Danger` =
 * `.btn-danger` with its icon (default trash); `confirm` → a small confirm
 * popover before the action fires. Fires `action` → `value` (or null) with
 * the entry's form `values` (§4.6).
 */
import { Show, createEffect, createSignal, createUniqueId, onCleanup } from 'solid-js';
import { useEntry } from '../actions';
import type { ButtonNode } from '../types';
import { IconView } from './icons';
import { InlineLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

const VARIANT: Record<string, string> = { Primary: 'btn-primary', Secondary: 'btn-secondary', Ghost: 'btn-ghost', Danger: 'btn-danger' };

export function Button(props: NodeProps<ButtonNode>) {
  const entry = useEntry();
  const [asking, setAsking] = createSignal(false);
  const popId = createUniqueId();
  let root!: HTMLSpanElement;
  const fire = () => {
    setAsking(false);
    entry.send(props.node.action, props.node.value ?? null, props.path, true);
  };
  const click = () => {
    if (props.node.disabled) return;
    if (props.node.confirm) setAsking((v) => !v);
    else fire();
  };
  // Outside press closes the confirm popover (listener only while it's open).
  createEffect(() => {
    if (!asking()) return;
    const outside = (e: PointerEvent) => {
      if (!root.contains(e.target as Node)) setAsking(false);
    };
    document.addEventListener('pointerdown', outside);
    onCleanup(() => document.removeEventListener('pointerdown', outside));
  });
  const icon = () => props.node.icon ?? (props.node.variant === 'Danger' ? 'trash' : undefined);
  return (
    <span ref={root} class="ui-button-wrap">
      <button
        type="button"
        class={cx('btn ui-btn', VARIANT[props.node.variant ?? 'Secondary'] ?? 'btn-secondary', baseStyle(props.node).class)}
        style={baseStyle(props.node).style}
        disabled={props.node.disabled}
        aria-haspopup={props.node.confirm ? 'dialog' : undefined}
        aria-expanded={props.node.confirm ? asking() : undefined}
        aria-controls={props.node.confirm && asking() ? popId : undefined}
        onClick={click}
      >
        <Show when={icon()}>
          <IconView icon={icon()} size={16} />
        </Show>
        <InlineLine line={props.node.label} />
      </button>
      <Show when={asking()}>
        <div
          id={popId}
          class="ui-confirm"
          role="dialog"
          aria-label="Confirm"
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              setAsking(false);
            }
          }}
        >
          <p class="ui-confirm-text">{props.node.confirm}</p>
          <div class="ui-confirm-actions">
            <button type="button" class="btn btn-ghost btn-sm" onClick={() => setAsking(false)}>
              Cancel
            </button>
            <button
              type="button"
              class={cx('btn btn-sm', props.node.variant === 'Danger' ? 'btn-danger' : 'btn-primary')}
              ref={(el) => queueMicrotask(() => el.focus())}
              onClick={fire}
            >
              <InlineLine line={props.node.label} />
            </button>
          </div>
        </div>
      </Show>
    </span>
  );
}
