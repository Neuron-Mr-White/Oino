/**
 * `Chart` (ui-protocol §4.5.1): SVG drawn at the measured pixel width (so
 * text never stretches), 12 rows tall (≥ 160 px on the phone, §13.6 rule 7);
 * polyline / scatter / bars per dataset with `vector-effect:
 * non-scaling-stroke`, axes with titles + labels, a legend; points are
 * LTTB-downsampled to ≤ 2 × the pixel width.
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { observeSize, rowPx } from '../layout/units';
import { tokenVar } from '../theme';
import type { Axis, ChartNode, Dataset } from '../types';
import { lttb } from './charts';
import { InlineLine, plainLine, spanOf } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

const PAD = { l: 44, r: 12, t: 10, b: 26 };

function seriesColor(d: Dataset, i: number): string {
  return tokenVar(d.style?.fg) ?? `var(--ui-c${(i % 12) + 1})`;
}

function bounds(axis: Axis | undefined, values: number[]): [number, number] {
  const b = axis?.bounds;
  if (Array.isArray(b) && b.length === 2 && Number.isFinite(b[0]) && Number.isFinite(b[1]) && b[1] !== b[0]) return [b[0], b[1]];
  if (!values.length) return [0, 1];
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  return lo === hi ? [lo - 1, hi + 1] : [lo, hi];
}

function ticks(axis: Axis | undefined, [lo, hi]: [number, number]): string[] {
  if (axis?.labels?.length) return axis.labels.map((l) => spanOf(l).content);
  const fmt = (v: number) => (Math.abs(v) >= 100 || Number.isInteger(v) ? String(Math.round(v)) : v.toFixed(1));
  return [fmt(lo), fmt((lo + hi) / 2), fmt(hi)];
}

export function Chart(props: NodeProps<ChartNode>) {
  let plot!: HTMLDivElement;
  const [box, setBox] = createSignal({ width: 0, height: 0 });
  onMount(() => onCleanup(observeSize(plot, setBox)));
  const W = () => (box().width > 0 ? box().width : 480);
  /** Auto: 12 rows (≥ 160 px); definite: whatever the plot box got (≥ 120 px). */
  const H = () => (props.mode === 'definite' && box().height > 0 ? Math.max(120, box().height) : Math.max(160, 12 * rowPx()));
  const datasets = () => (Array.isArray(props.node.datasets) ? props.node.datasets : []);
  const xb = createMemo(() => bounds(props.node.xAxis, datasets().flatMap((d) => (d.data ?? []).map((p) => p[0]))));
  const yb = createMemo(() => bounds(props.node.yAxis, datasets().flatMap((d) => (d.data ?? []).map((p) => p[1]))));
  const plotW = () => Math.max(10, W() - PAD.l - PAD.r);
  const plotH = () => Math.max(10, H() - PAD.t - PAD.b);
  const sx = (x: number) => PAD.l + ((x - xb()[0]) / (xb()[1] - xb()[0])) * plotW();
  const sy = (y: number) => PAD.t + plotH() - ((y - yb()[0]) / (yb()[1] - yb()[0])) * plotH();
  const pts = (d: Dataset) => lttb(d.data ?? [], Math.max(3, Math.floor(plotW() * 2)));
  const legend = () => (props.node.legendPosition === null ? undefined : (props.node.legendPosition ?? 'TopRight'));
  const named = () => datasets().filter((d) => d.name !== undefined);
  const xt = () => ticks(props.node.xAxis, xb());
  const yt = () => ticks(props.node.yAxis, yb());
  const label = () =>
    `Chart: ${datasets()
      .map((d) => `${plainLine(d.name ?? 'series')} (${d.data?.length ?? 0} points)`)
      .join(', ')}`;

  const Legend = () => (
    <Show when={legend() && named().length > 0}>
      <div class="ui-chart-legend" data-pos={legend()}>
        <For each={datasets()}>
          {(d, i) => (
            <Show when={d.name !== undefined}>
              <span class="ui-chart-key">
                <span class="ui-chart-swatch" style={{ background: seriesColor(d, i()) }} />
                <InlineLine line={d.name} />
              </span>
            </Show>
          )}
        </For>
      </div>
    </Show>
  );

  return (
    <div class={cx('ui-chart', baseStyle(props.node).class)} data-mode={props.mode} style={baseStyle(props.node).style} role="img" aria-label={label()}>
      <Show when={legend()?.startsWith('Top') || legend() === 'Left'}>
        <Legend />
      </Show>
      <div ref={plot} class="ui-chart-plot">
      <svg width="100%" height={H()} viewBox={`0 0 ${W()} ${H()}`} aria-hidden="true" class="ui-chart-svg">
        <g class="ui-chart-axes">
          <line x1={PAD.l} y1={PAD.t + plotH()} x2={PAD.l + plotW()} y2={PAD.t + plotH()} />
          <line x1={PAD.l} y1={PAD.t} x2={PAD.l} y2={PAD.t + plotH()} />
          <For each={yt()}>
            {(t, i) => {
              const y = () => PAD.t + plotH() - (i() / Math.max(1, yt().length - 1)) * plotH();
              return (
                <>
                  <line class="ui-chart-grid" x1={PAD.l} x2={PAD.l + plotW()} y1={y()} y2={y()} />
                  <text x={PAD.l - 6} y={y()} text-anchor="end" dominant-baseline="middle">
                    {t}
                  </text>
                </>
              );
            }}
          </For>
          <For each={xt()}>
            {(t, i) => {
              const x = () => PAD.l + (i() / Math.max(1, xt().length - 1)) * plotW();
              const anchor = () => (i() === 0 ? 'start' : i() === xt().length - 1 ? 'end' : 'middle');
              return (
                <text x={x()} y={PAD.t + plotH() + 16} text-anchor={anchor()}>
                  {t}
                </text>
              );
            }}
          </For>
        </g>
        <For each={datasets()}>
          {(d, i) => {
            const color = () => seriesColor(d, i());
            const p = () => pts(d);
            return (
              <Show
                when={(d.graphType ?? 'Scatter') === 'Line'}
                fallback={
                  <Show
                    when={d.graphType === 'Bar'}
                    fallback={<For each={p()}>{([x, y]) => <circle cx={sx(x)} cy={sy(y)} r={2.5} fill={color()} />}</For>}
                  >
                    <For each={p()}>
                      {([x, y]) => <line x1={sx(x)} x2={sx(x)} y1={sy(Math.max(yb()[0], 0))} y2={sy(y)} stroke={color()} stroke-width={3} vector-effect="non-scaling-stroke" />}
                    </For>
                  </Show>
                }
              >
                <polyline
                  points={p()
                    .map(([x, y]) => `${sx(x).toFixed(1)},${sy(y).toFixed(1)}`)
                    .join(' ')}
                  fill="none"
                  stroke={color()}
                  stroke-width={2}
                  stroke-linejoin="round"
                  stroke-linecap="round"
                  vector-effect="non-scaling-stroke"
                />
              </Show>
            );
          }}
        </For>
      </svg>
      </div>
      <Show when={props.node.xAxis?.title || props.node.yAxis?.title}>
        <div class="ui-chart-titles">
          <Show when={props.node.yAxis?.title}>
            <InlineLine line={props.node.yAxis!.title} class="ui-chart-ytitle" />
          </Show>
          <Show when={props.node.xAxis?.title}>
            <InlineLine line={props.node.xAxis!.title} class="ui-chart-xtitle" />
          </Show>
        </div>
      </Show>
      <Show when={legend() && !(legend()!.startsWith('Top') || legend() === 'Left')}>
        <Legend />
      </Show>
    </div>
  );
}
