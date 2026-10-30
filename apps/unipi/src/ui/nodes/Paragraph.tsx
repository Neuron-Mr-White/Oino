/**
 * `Paragraph` (ui-protocol §4.5.1): `<div class="ui-p">`, lines as block
 * rows, `white-space: pre-wrap` (or `pre` + horizontal scroll when
 * `wrap:false`); `scroll: [y, x]` sets the initial scroll in rows/columns.
 */
import { For, onMount } from 'solid-js';
import type { ParagraphNode } from '../types';
import { LineView, textOf } from './Text';
import { styleCss } from '../theme';
import { colPx, rowPx } from '../layout/units';
import { alignClass } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function Paragraph(props: NodeProps<ParagraphNode>) {
  let el!: HTMLDivElement;
  const text = () => textOf(props.node.text);
  const nowrap = () => props.node.wrap === false;
  onMount(() => {
    const s = props.node.scroll;
    if (Array.isArray(s)) {
      el.scrollTop = (s[0] ?? 0) * rowPx();
      el.scrollLeft = (s[1] ?? 0) * colPx();
    }
  });
  const css = () => {
    const base = baseStyle(props.node);
    const t = styleCss(text().style);
    return { class: cx(base.class, t.class), style: { ...base.style, ...t.style } };
  };
  return (
    <div
      ref={el}
      class={cx('ui-p', nowrap() && 'ui-nowrap', props.node.wrap && props.node.wrap.trim === false && 'ui-notrim', alignClass(props.node.alignment), css().class)}
      data-mode={props.mode}
      data-scrollbar={props.node.scrollbar ? '' : undefined}
      style={css().style}
    >
      <For each={text().lines}>{(l) => <LineView line={l} inheritAlign={text().alignment ?? props.node.alignment} />}</For>
    </div>
  );
}
