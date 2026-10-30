/**
 * `List` (ui-protocol §4.5.1): `role="listbox"` rows, 32 px desktop / 44 px
 * phone (`detail` adds a muted second line), badge right, icon left; selected
 * row = `surface-3` + 2 px shell left bar (+ `highlightStyle`); ↑↓ PgUp/PgDn
 * Home/End move, Enter activates, Space toggles (`multi`). Selection is local
 * state; `state.selected` wins when it changes. Virtualized above 100 items.
 *
 * Pointer default (spec names none): click selects; double-click activates;
 * a list with only `onActivate` activates on click; on the phone a tap selects
 * and activates (no double-tap on touch).
 */
import { For, Show, createMemo, createSignal, type JSX } from 'solid-js';
import { useEntry } from '../actions';
import { useLocal } from '../localState';
import { shell } from '../layout/units';
import { styleCss } from '../theme';
import type { ListItem, ListItemObj, ListNode, Text } from '../types';
import { VIRTUAL_THRESHOLD, VirtualWindow, useVirtual } from '../virtual';
import { IconView } from './icons';
import { InlineLine, TextView } from './Text';
import { baseStyle, cx, itemKey, type NodeProps } from './props';

const isFull = (it: ListItem): it is ListItemObj => !!it && typeof it === 'object' && !Array.isArray(it) && 'content' in it;

export function listItemObj(it: ListItem): ListItemObj {
  return isFull(it) ? it : { content: it as Text };
}

/** Moves a selection index by a key; returns undefined when the key isn't a navigation key. */
export function navIndex(key: string, cur: number, count: number, page = 10): number | undefined {
  if (count === 0) return undefined;
  switch (key) {
    case 'ArrowDown':
      return Math.min(count - 1, cur < 0 ? 0 : cur + 1);
    case 'ArrowUp':
      return Math.max(0, cur < 0 ? 0 : cur - 1);
    case 'PageDown':
      return Math.min(count - 1, Math.max(0, cur) + page);
    case 'PageUp':
      return Math.max(0, cur - page);
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    default:
      return undefined;
  }
}

export function List(props: NodeProps<ListNode>) {
  const entry = useEntry();
  const items = createMemo(() => {
    const raw = Array.isArray(props.node.items) ? props.node.items : [];
    const keyed = raw.map((it, i) => ({ key: itemKey(it, i), item: listItemObj(it) }));
    return props.node.direction === 'BottomToTop' ? keyed.reverse() : keyed;
  });
  const multi = () => !!props.node.multi;
  const [selected, setSelected] = useLocal<string[], string | string[]>(entry.entryId, props.path, 'selected', {
    prop: () => props.node.state?.selected,
    fromProp: (p) => (Array.isArray(p) ? p : p === undefined ? [] : [p]),
    init: () => [],
  });
  const [cursor, setCursor] = createSignal<string | undefined>(selected()[0]);
  const isSel = (k: string) => selected().includes(k);
  const indexOf = (k: string | undefined) => (k === undefined ? -1 : items().findIndex((x) => x.key === k));

  const select = (k: string) => {
    const it = items().find((x) => x.key === k);
    if (!it || it.item.disabled) return;
    setCursor(k);
    if (multi()) {
      const next = isSel(k) ? selected().filter((x) => x !== k) : [...selected(), k];
      setSelected(next);
      entry.send(props.node.onSelect, next, props.path);
    } else {
      if (selected()[0] === k) return;
      setSelected([k]);
      entry.send(props.node.onSelect, k, props.path);
    }
  };
  const activate = (k: string) => {
    const it = items().find((x) => x.key === k);
    if (!it || it.item.disabled) return;
    entry.send(props.node.onActivate, k, props.path);
  };
  const onRowClick = (k: string) => {
    if (props.node.onActivate && !props.node.onSelect && !multi()) {
      setCursor(k);
      setSelected([k]);
      activate(k);
      return;
    }
    select(k);
    if (shell() === 'phone' && !multi()) activate(k);
  };

  let scroller: HTMLDivElement | undefined;
  const [scrollEl, setScrollEl] = createSignal<HTMLElement>();
  const virtual = () => items().length > VIRTUAL_THRESHOLD;
  const rowH = (it: ListItemObj) => (shell() === 'desktop' ? (it.detail ? 52 : 32) : it.detail ? 56 : 44);
  const virt = useVirtual({
    count: () => (virtual() ? items().length : 0),
    scroller: scrollEl,
    estimate: (i) => rowH(items()[i]?.item ?? { content: '' }),
    key: (i) => items()[i]?.key ?? i,
  });

  const scrollTo = (i: number) => {
    if (virtual()) virt.scrollToIndex(i, { align: 'auto' });
    else scroller?.querySelector<HTMLElement>(`[data-index="${i}"]`)?.scrollIntoView?.({ block: 'nearest' });
  };

  const onKey = (e: KeyboardEvent) => {
    const list = items();
    const cur = indexOf(cursor() ?? selected()[0]);
    const next = navIndex(e.key, cur, list.length);
    if (next !== undefined) {
      e.preventDefault();
      // Skip disabled rows in the direction of travel.
      let i = next;
      const step = next >= cur ? 1 : -1;
      while (list[i]?.item.disabled && i + step >= 0 && i + step < list.length) i += step;
      const k = list[i]?.key;
      if (k === undefined) return;
      if (multi()) setCursor(k);
      else select(k);
      scrollTo(i);
      return;
    }
    const k = cursor() ?? selected()[0];
    if (k === undefined) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      activate(k);
    } else if (e.key === ' ' && multi()) {
      e.preventDefault();
      select(k);
    }
  };

  const symbol = () => props.node.highlightSymbol;
  const spacing = () => props.node.highlightSpacing ?? 'WhenSelected';
  const showSymbolCol = () => !!symbol() && (spacing() === 'Always' || (spacing() === 'WhenSelected' && selected().length > 0));
  const highlight = () => styleCss(props.node.highlightStyle);

  const Row = (p: { index: number }): JSX.Element => {
    const row = () => items()[p.index];
    const k = () => row()?.key ?? '';
    const it = () => row()?.item ?? { content: '' };
    const sel = () => isSel(k());
    const css = () => styleCss(it().style);
    return (
      <div
        role="option"
        id={`${entry.entryId}:${props.path.join('/')}:${k()}`}
        class={cx('ui-row', css().class, sel() && highlight().class)}
        data-index={p.index}
        data-selected={sel() ? '' : undefined}
        data-cursor={cursor() === k() ? '' : undefined}
        data-disabled={it().disabled ? '' : undefined}
        data-detail={it().detail ? '' : undefined}
        aria-selected={sel()}
        aria-disabled={it().disabled ? true : undefined}
        style={{ ...css().style, ...(sel() ? highlight().style : {}) }}
        onClick={() => onRowClick(k())}
        onDblClick={() => props.node.onSelect && activate(k())}
      >
        <Show when={showSymbolCol()}>
          <span class="ui-row-symbol" aria-hidden="true">
            {sel() ? symbol() : ''}
          </span>
        </Show>
        <Show when={it().icon}>
          <IconView icon={it().icon} size={16} class="ui-row-icon" />
        </Show>
        <span class="ui-row-main">
          <TextView text={it().content} class="ui-row-content" />
          <Show when={it().detail}>
            <InlineLine line={it().detail} class="ui-row-detail" />
          </Show>
        </span>
        <Show when={it().badge}>
          <InlineLine line={it().badge} class="ui-row-badge" />
        </Show>
      </div>
    );
  };

  return (
    <div
      ref={(el) => {
        scroller = el;
        setScrollEl(el);
      }}
      class={cx('ui-list', baseStyle(props.node).class)}
      role="listbox"
      tabindex="0"
      aria-multiselectable={multi() ? true : undefined}
      aria-activedescendant={cursor() ? `${entry.entryId}:${props.path.join('/')}:${cursor()}` : undefined}
      data-mode={props.mode}
      data-virtual={virtual() ? '' : undefined}
      data-scrollbar={props.node.scrollbar ? '' : undefined}
      data-direction={props.node.direction === 'BottomToTop' ? 'btt' : undefined}
      style={baseStyle(props.node).style}
      onKeyDown={onKey}
    >
      <Show when={virtual()} fallback={<For each={items()}>{(_, i) => <Row index={i()} />}</For>}>
        <VirtualWindow virt={virt}>{(i) => <Row index={i} />}</VirtualWindow>
      </Show>
    </div>
  );
}
