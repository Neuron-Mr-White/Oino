/**
 * `Gauge` (ui-protocol §4.5.1): label centred above an 8 px rounded bar, fill
 * `gaugeStyle.fg` (default `accent`); `role="progressbar"` with the percentage.
 */
import { Show } from 'solid-js';
import { tokenVar } from '../theme';
import type { GaugeNode } from '../types';
import { SpanView, spanOf } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function clampRatio(r: unknown): number {
  const n = typeof r === 'number' && Number.isFinite(r) ? r : 0;
  return Math.min(1, Math.max(0, n));
}

export function gaugeRatio(node: { ratio?: number; percent?: number }): number {
  if (typeof node.ratio === 'number') return clampRatio(node.ratio);
  if (typeof node.percent === 'number') return clampRatio(node.percent / 100);
  return 0;
}

export function Gauge(props: NodeProps<GaugeNode>) {
  const ratio = () => gaugeRatio(props.node);
  const pct = () => Math.round(ratio() * 100);
  const label = () => (props.node.label !== undefined ? spanOf(props.node.label).content : `${pct()}%`);
  const fill = () => tokenVar(props.node.gaugeStyle?.fg) ?? 'var(--ui-accent)';
  const track = () => tokenVar(props.node.gaugeStyle?.bg) ?? 'var(--ui-surface3)';
  return (
    <div
      class={cx('ui-gauge', baseStyle(props.node).class)}
      style={baseStyle(props.node).style}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct()}
      aria-valuetext={label()}
    >
      <div class="ui-gauge-label">
        <Show when={props.node.label !== undefined} fallback={<span>{label()}</span>}>
          <SpanView span={props.node.label!} />
        </Show>
      </div>
      <div class="ui-gauge-track" style={{ background: track() }}>
        <div class="ui-gauge-fill" style={{ width: `${ratio() * 100}%`, background: fill() }} />
      </div>
    </div>
  );
}
