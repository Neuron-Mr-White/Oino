/**
 * `KeyHints` (ui-protocol §4.5.2): desktop — a row of `kbd` chips + labels;
 * phone (§13.6 rule 5) — hidden unless a hint has an `action`, then only
 * those show, as tappable chips. A hint with `action` fires it (value null).
 */
import { For, Show } from 'solid-js';
import { useEntry } from '../actions';
import { shell } from '../layout/units';
import type { Hint, KeyHintsNode } from '../types';
import { baseStyle, cx, type NodeProps } from './props';

/** pi KeyId → display chips: `ctrl+s` → ["Ctrl", "S"]. */
export function keyChips(keys: string): string[] {
  const names: Record<string, string> = { ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', meta: 'Cmd', cmd: 'Cmd', super: 'Super', enter: 'Enter', escape: 'Esc', esc: 'Esc', tab: 'Tab', space: 'Space', up: '↑', down: '↓', left: '←', right: '→', pageup: 'PgUp', pagedown: 'PgDn', backspace: '⌫', delete: 'Del', home: 'Home', end: 'End' };
  return keys
    .split(/[+-](?=.)/)
    .filter(Boolean)
    .map((k) => names[k.toLowerCase()] ?? (k.length === 1 ? k.toUpperCase() : k[0]!.toUpperCase() + k.slice(1)));
}

export function KeyHints(props: NodeProps<KeyHintsNode>) {
  const entry = useEntry();
  const hints = () => {
    const all = Array.isArray(props.node.hints) ? props.node.hints : [];
    return shell() === 'phone' ? all.filter((h) => h.action) : all;
  };
  const Chips = (p: { hint: Hint }) => (
    <>
      <span class="ui-keys" aria-hidden={shell() === 'phone' ? 'true' : undefined}>
        <For each={keyChips(p.hint.keys)}>{(k) => <kbd class="kbd">{k}</kbd>}</For>
      </span>
      <span class="ui-hint-label">{p.hint.label}</span>
    </>
  );
  return (
    <Show when={hints().length > 0}>
      <div class={cx('ui-keyhints', baseStyle(props.node).class)} style={baseStyle(props.node).style} role="group" aria-label="Keys">
        <For each={hints()}>
          {(h) => (
            <Show
              when={h.action}
              fallback={
                <span class="ui-hint">
                  <Chips hint={h} />
                </span>
              }
            >
              <button type="button" class="ui-hint ui-hint-action" onClick={() => entry.send(h.action, null, props.path)}>
                <Chips hint={h} />
              </button>
            </Show>
          )}
        </For>
      </div>
    </Show>
  );
}
