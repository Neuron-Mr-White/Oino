/**
 * `LineGauge` (ui-protocol §4.5.1): label left + a 4 px bar (`filledStyle` /
 * `unfilledStyle`); `role="progressbar"` with the percentage.
 */
import { Show } from 'solid-js';
import { tokenVar } from '../theme';
import type { LineGaugeNode } from '../types';
import { clampRatio } from './Gauge';
import { InlineLine, plainLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function LineGauge(props: NodeProps<LineGaugeNode>) {
  const ratio = () => clampRatio(props.node.ratio);
  const pct = () => Math.round(ratio() * 100);
  const fill = () => tokenVar(props.node.filledStyle?.fg) ?? 'var(--ui-accent)';
  const track = () => tokenVar(props.node.unfilledStyle?.fg) ?? 'var(--ui-surface3)';
  return (
    <div
      class={cx('ui-linegauge', baseStyle(props.node).class)}
      data-set={(props.node.lineSet ?? 'Normal').toLowerCase()}
      style={baseStyle(props.node).style}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct()}
      aria-valuetext={props.node.label !== undefined ? plainLine(props.node.label) : `${pct()}%`}
    >
      <span class="ui-linegauge-label">
        <Show when={props.node.label !== undefined} fallback={`${pct()}%`}>
          <InlineLine line={props.node.label} />
        </Show>
      </span>
      <div class="ui-linegauge-track" style={{ background: track() }}>
        <div class="ui-linegauge-fill" style={{ width: `${ratio() * 100}%`, background: fill() }} />
      </div>
    </div>
  );
}
