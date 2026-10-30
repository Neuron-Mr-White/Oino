/**
 * `Input` (ui-protocol §4.5.2): `<input>` / `<textarea>` (44 px,
 * `--radius-input`). Typed text is local state (`value` wins when it
 * changes); `onChange` → string debounced 150 ms; `onSubmit` → string on
 * Enter (Ctrl/Cmd+Enter when multiline) with the entry's form `values`.
 */
import { Show, createUniqueId, onCleanup } from 'solid-js';
import { useEntry, useFormValue } from '../actions';
import { useLocal } from '../localState';
import type { InputNode } from '../types';
import { InlineLine, plainLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

const DEBOUNCE_MS = 150;

export function Input(props: NodeProps<InputNode>) {
  const entry = useEntry();
  const id = createUniqueId();
  const [value, setValue] = useLocal<string>(entry.entryId, props.path, 'value', {
    prop: () => props.node.value,
    init: () => '',
  });
  useFormValue(props.node.key, value);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => timer && clearTimeout(timer));
  const onInput = (v: string) => {
    setValue(v);
    if (!props.node.onChange) return;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => entry.send(props.node.onChange, value(), props.path), DEBOUNCE_MS);
  };
  const submit = () => {
    if (timer) {
      clearTimeout(timer);
      timer = undefined;
      entry.send(props.node.onChange, value(), props.path);
    }
    entry.send(props.node.onSubmit, value(), props.path, true);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Enter' || e.isComposing || !props.node.onSubmit) return;
    if (props.node.multiline && !(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    submit();
  };
  const label = () => (props.node.label !== undefined ? plainLine(props.node.label) : undefined);
  return (
    <div class={cx('ui-field', baseStyle(props.node).class)} style={baseStyle(props.node).style}>
      <Show when={props.node.label !== undefined}>
        <label class="ui-field-label" for={id}>
          <InlineLine line={props.node.label} />
        </label>
      </Show>
      <Show
        when={props.node.multiline}
        fallback={
          <input
            id={id}
            class="ui-input"
            type={props.node.secret ? 'password' : 'text'}
            value={value()}
            placeholder={props.node.placeholder}
            maxLength={props.node.maxLength}
            disabled={props.node.disabled}
            autocomplete={props.node.secret ? 'off' : undefined}
            aria-label={label() ? undefined : (props.node.placeholder ?? props.node.key)}
            onInput={(e) => onInput(e.currentTarget.value)}
            onKeyDown={onKey}
          />
        }
      >
        <textarea
          id={id}
          class="ui-input ui-textarea"
          rows={props.node.rows || 3}
          value={value()}
          placeholder={props.node.placeholder}
          maxLength={props.node.maxLength}
          disabled={props.node.disabled}
          aria-label={label() ? undefined : (props.node.placeholder ?? props.node.key)}
          data-secret={props.node.secret ? '' : undefined}
          onInput={(e) => onInput(e.currentTarget.value)}
          onKeyDown={onKey}
        />
      </Show>
    </div>
  );
}
