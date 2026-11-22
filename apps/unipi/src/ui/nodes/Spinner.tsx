/**
 * `Spinner` (ui-protocol §4.5.2): CSS-animated ring (no JS timer);
 * `prefers-reduced-motion` → a static dot (`ui.css`).
 */
import { Show } from 'solid-js';
import type { SpinnerNode } from '../types';
import { InlineLine, plainLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function Spinner(props: NodeProps<SpinnerNode>) {
  return (
    <span
      class={cx('ui-spinner', baseStyle(props.node).class)}
      style={baseStyle(props.node).style}
      role="status"
      aria-label={props.node.label !== undefined ? plainLine(props.node.label) : 'Working'}
    >
      <span class="ui-spinner-ring" aria-hidden="true" />
      <Show when={props.node.label !== undefined}>
        <InlineLine line={props.node.label} class="ui-spinner-label" />
      </Show>
    </span>
  );
}
