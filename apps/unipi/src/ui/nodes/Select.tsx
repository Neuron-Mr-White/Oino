/**
 * `Select` (ui-protocol §4.5.2): a Kobalte select — a popover listbox on the
 * desktop, a bottom sheet on the phone (`data-phone` content, positioned by
 * `ui.css`). Options show their `description` as a muted second line. Local
 * state, `value` wins when it changes; `onChange` → value.
 */
import { Show } from 'solid-js';
import { Select as KSelect } from '@kobalte/core/select';
import { Check, ChevronsUpDown } from 'lucide-solid';
import { useEntry, useFormValue } from '../actions';
import { useLocal } from '../localState';
import { shell } from '../layout/units';
import type { SelectNode, SelectOption } from '../types';
import { InlineLine, plainLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function Select(props: NodeProps<SelectNode>) {
  const entry = useEntry();
  const [value, setValue] = useLocal<string | undefined>(entry.entryId, props.path, 'value', {
    prop: () => props.node.value,
    init: () => undefined,
  });
  useFormValue(props.node.key, value);
  const options = () => (Array.isArray(props.node.options) ? props.node.options : []);
  const current = () => options().find((o) => o.value === value()) ?? null;
  const change = (o: SelectOption | null) => {
    if (!o || o.value === value()) return;
    setValue(o.value);
    entry.send(props.node.onChange, o.value, props.path);
  };
  return (
    <div class={cx('ui-select', baseStyle(props.node).class)} style={baseStyle(props.node).style}>
      <KSelect<SelectOption>
        options={options()}
        optionValue="value"
        optionTextValue={(o) => plainLine(o.label)}
        value={current()}
        onChange={change}
        disabled={props.node.disabled}
        placeholder={props.node.placeholder ?? 'Choose…'}
        gutter={6}
        sameWidth
        itemComponent={(p) => (
          <KSelect.Item item={p.item} class="ui-select-item">
            <span class="ui-select-item-main">
              <KSelect.ItemLabel class="ui-select-item-label">
                <InlineLine line={p.item.rawValue.label} />
              </KSelect.ItemLabel>
              <Show when={p.item.rawValue.description}>
                <KSelect.ItemDescription class="ui-select-item-desc">{p.item.rawValue.description}</KSelect.ItemDescription>
              </Show>
            </span>
            <KSelect.ItemIndicator class="ui-select-check">
              <Check size={16} aria-hidden="true" />
            </KSelect.ItemIndicator>
          </KSelect.Item>
        )}
      >
        <KSelect.Trigger class="ui-select-trigger" aria-label={props.node.placeholder ?? props.node.key}>
          <KSelect.Value<SelectOption> class="ui-select-value">{(s) => <InlineLine line={s.selectedOption().label} />}</KSelect.Value>
          <KSelect.Icon class="ui-select-icon">
            <ChevronsUpDown size={16} aria-hidden="true" />
          </KSelect.Icon>
        </KSelect.Trigger>
        <KSelect.Portal>
          <KSelect.Content class="ui-select-content" data-phone={shell() === 'phone' ? '' : undefined}>
            <KSelect.Listbox class="ui-select-listbox" />
          </KSelect.Content>
        </KSelect.Portal>
      </KSelect>
    </div>
  );
}
