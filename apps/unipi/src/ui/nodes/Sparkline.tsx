/**
 * `Sparkline` (ui-protocol §4.5.1): inline SVG bars in the node's `fg`
 * (default `accent`), height = 3 rows × `--ui-row`; `null` = absent; data
 * bucketed to ≤ 2 × the measured pixel width.
 */
import { For, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { observeSize } from '../layout/units';
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
  const n = () => Math.max(1, data().length);
  const summary = () => {
    const vals = data().filter((v): v is number => typeof v === 'number');
    return vals.length ? `Sparkline, ${vals.length} values, last ${vals[vals.length - 1]}, max ${Math.max(...vals)}` : 'Sparkline, no data';
  };
  return (
    <div ref={el} class={cx('ui-sparkline', baseStyle(props.node).class)} style={baseStyle(props.node).style} role="img" aria-label={summary()}>
      <svg viewBox={`0 0 ${n()} 100`} preserveAspectRatio="none" width="100%" height="100%" aria-hidden="true">
        <For each={data()}>
          {(v, i) => {
            const h = () => (typeof v === 'number' ? Math.max(v > 0 ? 2 : 0, (Math.min(v, max()) / max()) * 100) : 0);
            return typeof v === 'number' ? (
              <rect x={i() + 0.12} width={0.76} y={100 - h()} height={h()} rx={0} fill={fill()} />
            ) : (
              <rect x={i() + 0.3} width={0.4} y={98} height={2} fill="var(--ui-dim)" />
            );
          }}
        </For>
      </svg>
    </div>
  );
}
