/**
 * `Tabs` (ui-protocol §4.5.1): a segmented strip of `--chip-h` (38 px) chips,
 * horizontally scrollable on the phone (§13.6 rule 4); the selected pane
 * renders below and panes switch locally (no round-trip; `selected` wins when
 * it changes); `onSelect` → tab key. ←/→ cycle (roving tabindex).
 */
import { For, Show, createMemo } from 'solid-js';
import { useEntry } from '../actions';
import { useLocal } from '../localState';
import { NodeView } from '../NodeView';
import { styleCss } from '../theme';
import type { TabsNode } from '../types';
import { InlineLine, SpanView } from './Text';
import { baseStyle, childPath, cx, type NodeProps } from './props';

export function Tabs(props: NodeProps<TabsNode>) {
  const entry = useEntry();
  const titles = createMemo(() => (Array.isArray(props.node.titles) ? props.node.titles : []));
  const [selected, setSelected] = useLocal<string | undefined>(entry.entryId, props.path, 'tab', {
    prop: () => props.node.selected,
    init: () => titles()[0]?.key,
  });
  const current = () => (titles().some((t) => t.key === selected()) ? selected() : titles()[0]?.key);
  const hl = () => styleCss(props.node.highlightStyle);
  const tabIds = (k: string) => `${entry.entryId}:${props.path.join('/')}:tab:${k}`;
  let strip!: HTMLDivElement;

  const choose = (k: string, focus = false) => {
    if (focus) [...strip.querySelectorAll<HTMLElement>('[role=tab]')].find((b) => b.dataset.key === k)?.focus();
    if (k === current()) return;
    setSelected(k);
    entry.send(props.node.onSelect, k, props.path);
  };
  const onKey = (e: KeyboardEvent) => {
    const list = titles();
    const i = list.findIndex((t) => t.key === current());
    let next: number | undefined;
    if (e.key === 'ArrowRight') next = (i + 1) % list.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + list.length) % list.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = list.length - 1;
    if (next === undefined || !list[next]) return;
    e.preventDefault();
    choose(list[next]!.key, true);
  };
  const pane = () => {
    const k = current();
    return k !== undefined ? props.node.panes?.[k] : undefined;
  };

  return (
    <div class={cx('ui-tabs', baseStyle(props.node).class)} data-mode={props.mode} style={baseStyle(props.node).style}>
      <div ref={strip} class="ui-tabs-strip" role="tablist" onKeyDown={onKey}>
        <For each={titles()}>
          {(t, i) => {
            const sel = () => t.key === current();
            return (
              <>
                <Show when={i() > 0 && props.node.divider !== undefined}>
                  <span class="ui-tabs-divider" aria-hidden="true">
                    <SpanView span={props.node.divider!} />
                  </span>
                </Show>
                <button
                  type="button"
                  role="tab"
                  id={tabIds(t.key)}
                  data-key={t.key}
                  class={cx('ui-tab', sel() && hl().class)}
                  style={sel() ? hl().style : undefined}
                  aria-selected={sel()}
                  tabindex={sel() ? 0 : -1}
                  onClick={() => choose(t.key)}
                >
                  <InlineLine line={t.title} />
                  <Show when={t.badge}>
                    <InlineLine line={t.badge} class="ui-tab-badge" />
                  </Show>
                </button>
              </>
            );
          }}
        </For>
      </div>
      <Show when={pane()}>
        <div class="ui-tabs-pane" role="tabpanel" aria-labelledby={current() ? tabIds(current()!) : undefined} data-mode={props.mode}>
          <Show when={current()} keyed>
            {(k) => <NodeView node={props.node.panes?.[k]} path={childPath(props.path, k)} mode={props.mode} />}
          </Show>
        </div>
      </Show>
    </div>
  );
}
