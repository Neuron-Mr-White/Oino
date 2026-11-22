/**
 * `Sparkline` (ui-protocol §4.5.1): inline SVG bars in the node's `fg`
 * (default `accent`), height = 3 rows × `--ui-row`; `null` = absent; data
 * bucketed to ≤ 2 × the measured pixel width.
 */
import { For, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { colPx, observeSize } from '../layout/units';
import { tokenVar } from '../theme';
import type { SparklineNode } from '../types';
import { bucketMax } from './charts';
import { baseStyle, cx, type NodeProps } from './props';

export function Sparkline(props: NodeProps<SparklineNode>) {
  let el!: HTMLDivElement;
  const [width, setWidth] = createSignal(0);
  onMount(() => onCleanup(observeSize(el, (s) => setWidth(s.width))));
  const data = createMemo(() => {
    const raw = Array.isArray(props.node.data) ? props.node.data : [];
    const series = width() > 0 ? bucketMax(raw, Math.ceil(width() * 2)) : raw;
    return props.node.direction === 'RightToLeft' ? [...series].reverse() : series;
  });
  const max = () => {
    if (typeof props.node.max === 'number' && props.node.max > 0) return props.node.max;
    let m = 0;
    for (const v of data()) if (typeof v === 'number' && v > m) m = v;
    return m || 1;
  };
  const fill = () => tokenVar(props.node.style?.fg) ?? 'var(--ui-accent)';
  /** ratatui draws one cell per value: bars are one column wide (2 px gap), squeezed only when they don't fit. */
  const W = () => (width() > 0 ? width() : 320);
  const slot = () => Math.min(colPx(), W() / Math.max(1, data().length));
  const x0 = () => (props.node.direction === 'RightToLeft' ? W() - slot() * data().length : 0);
  const summary = () => {
    const vals = data().filter((v): v is number => typeof v === 'number');
    return vals.length ? `Sparkline, ${vals.length} values, last ${vals[vals.length - 1]}, max ${Math.max(...vals)}` : 'Sparkline, no data';
  };
  return (
    <div ref={el} class={cx('ui-sparkline', baseStyle(props.node).class)} style={baseStyle(props.node).style} role="img" aria-label={summary()}>
      <svg viewBox={`0 0 ${W()} 100`} preserveAspectRatio="none" width="100%" height="100%" aria-hidden="true">
        <For each={data()}>
          {(v, i) => {
            const h = () => (typeof v === 'number' ? Math.max(v > 0 ? 3 : 0, (Math.min(v, max()) / max()) * 100) : 0);
            const x = () => x0() + i() * slot();
            const w = () => Math.max(1, slot() - (slot() > 4 ? 2 : 0));
            return typeof v === 'number' ? (
              <rect x={x()} width={w()} y={100 - h()} height={h()} fill={fill()} />
            ) : (
              <rect x={x()} width={w()} y={98} height={2} fill="var(--ui-dim)" />
            );
          }}
        </For>
      </svg>
    </div>
  );
}
