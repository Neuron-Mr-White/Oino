/**
 * `BarChart` (ui-protocol §4.5.1). Vertical: groups of bars over a baseline,
 * value above each bar, bar label + group label below. Horizontal: labelled
 * bar rows. Bars are boxes (crisp text at any width — SVG text would stretch
 * under `preserveAspectRatio="none"`); `barWidth`/`barGap`/`groupGap` in
 * columns. A bar with `onSelect` is a button → `"<groupIndex>:<barIndex>"`.
 */
import { For, Show, type JSX } from 'solid-js';
import { useEntry } from '../actions';
import { styleCss, tokenVar } from '../theme';
import type { Bar, BarChartNode } from '../types';
import { InlineLine, plainLine } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

export function BarChart(props: NodeProps<BarChartNode>) {
  const entry = useEntry();
  const groups = () => (Array.isArray(props.node.data) ? props.node.data : []);
  const max = () => {
    if (typeof props.node.max === 'number' && props.node.max > 0) return props.node.max;
    let m = 0;
    for (const g of groups()) for (const b of g.bars ?? []) if (b.value > m) m = b.value;
    return m || 1;
  };
  const horizontal = () => props.node.direction === 'Horizontal';
  const color = (b: Bar) => tokenVar(b.style?.fg) ?? tokenVar(props.node.barStyle?.fg) ?? 'var(--ui-accent)';
  const valueText = (b: Bar) => b.textValue ?? String(b.value);
  const pick = (gi: number, bi: number) => entry.send(props.node.onSelect, `${gi}:${bi}`, props.path);
  const interactive = () => !!props.node.onSelect;
  const valueCss = (b: Bar) => styleCss(b.valueStyle ?? props.node.valueStyle);
  const labelCss = () => styleCss(props.node.labelStyle);
  const unit = (n: number | undefined, d: number) => `calc(${n ?? d} * var(--ui-col))`;

  const barEl = (b: Bar, gi: number, bi: number, inner: JSX.Element) =>
    interactive() ? (
      <button type="button" class="ui-bar-hit" aria-label={`${plainLine(b.label ?? '')} ${valueText(b)}`.trim()} onClick={() => pick(gi, bi)}>
        {inner}
      </button>
    ) : (
      inner
    );

  return (
    <div
      class={cx('ui-barchart', baseStyle(props.node).class)}
      data-dir={horizontal() ? 'h' : 'v'}
      style={{ ...baseStyle(props.node).style, '--ui-group-gap': unit(props.node.groupGap, 1), '--ui-bar-gap': unit(props.node.barGap, 1) } as JSX.CSSProperties}
      role="group"
      aria-label="Bar chart"
    >
      <Show
        when={horizontal()}
        fallback={
          <div class="ui-bars-v">
            <For each={groups()}>
              {(g, gi) => (
                <div class="ui-bar-group">
                  <div class="ui-bar-cols">
                    <For each={g.bars ?? []}>
                      {(b, bi) =>
                        barEl(
                          b,
                          gi(),
                          bi(),
                          <div class="ui-bar-col" style={{ width: `max(${unit(props.node.barWidth, 1)}, 10px)` }}>
                            <span class={cx('ui-bar-value', valueCss(b).class)} style={valueCss(b).style}>
                              {valueText(b)}
                            </span>
                            <div class="ui-bar" style={{ height: `${(Math.min(b.value, max()) / max()) * 100}%`, background: color(b) }} />
                            <Show when={b.label}>
                              <InlineLine line={b.label} class={cx('ui-bar-label', labelCss().class)} />
                            </Show>
                          </div>,
                        )
                      }
                    </For>
                  </div>
                  <Show when={g.label}>
                    <InlineLine line={g.label} class={cx('ui-bar-group-label', labelCss().class)} />
                  </Show>
                </div>
              )}
            </For>
          </div>
        }
      >
        <div class="ui-bars-h">
          <For each={groups()}>
            {(g, gi) => (
              <div class="ui-bar-group">
                <Show when={g.label}>
                  <InlineLine line={g.label} class={cx('ui-bar-group-label', labelCss().class)} />
                </Show>
                <For each={g.bars ?? []}>
                  {(b, bi) =>
                    barEl(
                      b,
                      gi(),
                      bi(),
                      <div class="ui-bar-row">
                        <InlineLine line={b.label ?? ''} class={cx('ui-bar-label', labelCss().class)} />
                        <div class="ui-bar-track">
                          <div class="ui-bar" style={{ width: `${(Math.min(b.value, max()) / max()) * 100}%`, background: color(b) }} />
                        </div>
                        <span class={cx('ui-bar-value', valueCss(b).class)} style={valueCss(b).style}>
                          {valueText(b)}
                        </span>
                      </div>,
                    )
                  }
                </For>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
