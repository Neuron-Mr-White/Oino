/**
 * `Toggle` (ui-protocol §4.5.2): the app's one switch (`SwitchField`, Kobalte)
 * with its label as a row; local state, `value` wins when it changes;
 * `onChange` → bool.
 */
import { SwitchField } from '../../components/Switch';
import { useEntry, useFormValue } from '../actions';
import { useLocal } from '../localState';
import type { ToggleNode } from '../types';
import { InlineLine, plainLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function Toggle(props: NodeProps<ToggleNode>) {
  const entry = useEntry();
  const [on, setOn] = useLocal<boolean>(entry.entryId, props.path, 'value', {
    prop: () => props.node.value,
    init: () => false,
  });
  useFormValue(props.node.key, on);
  const change = (v: boolean) => {
    setOn(v);
    entry.send(props.node.onChange, v, props.path);
  };
  return (
    <div class={cx('ui-toggle', props.node.disabled && 'ui-disabled', baseStyle(props.node).class)} style={baseStyle(props.node).style}>
      <span class="ui-toggle-label" onClick={() => !props.node.disabled && change(!on())}>
        <InlineLine line={props.node.label} />
      </span>
      <SwitchField checked={on()} onChange={change} label={plainLine(props.node.label)} disabled={props.node.disabled} />
    </div>
  );
}
