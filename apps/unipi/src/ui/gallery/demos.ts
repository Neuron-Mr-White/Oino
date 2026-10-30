/**
 * Composite demo trees for the gallery (definite-mode solver, realistic
 * extension panels). Plain data — the same shapes extensions send.
 */
import type { UiNode } from '../types';

/** A dashboard in a definite box: header / body / footer rows, a 30 % / Fill split. */
export const DASHBOARD: UiNode = {
  t: 'Layout',
  direction: 'Vertical',
  constraints: [{ Length: 3 }, { Fill: 1 }, { Length: 1 }],
  children: [
    {
      t: 'Block',
      key: 'head',
      borders: 'ALL',
      borderType: 'Rounded',
      title: [{ content: 'Long-horizon', alignment: 'Left' }, { content: [{ content: ' running ', style: { fg: 'success', bg: 'successBg' } }], alignment: 'Right' }],
      child: {
        t: 'LineGauge',
        ratio: 0.62,
        label: [{ content: 'Step 5 of 8 ', style: { fg: 'muted' } }, { content: '62%', style: { addModifier: ['BOLD'] } }],
        filledStyle: { fg: 'accent' },
      },
    },
    {
      t: 'Layout',
      key: 'body',
      direction: 'Horizontal',
      constraints: [{ Percentage: 34 }, { Fill: 1 }],
      spacing: 1,
      children: [
        {
          t: 'Block',
          key: 'steps',
          title: 'Steps',
          borders: 'ALL',
          borderType: 'Rounded',
          child: {
            t: 'List',
            items: [
              { key: 's1', content: 'Plan the migration', icon: 'check', detail: 'done · 2m', style: { fg: 'muted' } },
              { key: 's2', content: 'Port the solver', icon: 'check', detail: 'done · 9m', style: { fg: 'muted' } },
              { key: 's3', content: 'Node components', icon: 'play', detail: 'running · 14m', badge: [{ content: ' now ', style: { fg: 'accent', bg: 'accentBg' } }] },
              { key: 's4', content: 'Goldens', icon: 'clock', detail: 'queued' },
              { key: 's5', content: 'Gallery screenshots', icon: 'clock', detail: 'queued' },
            ],
            state: { selected: 's3' },
            onSelect: 'pick-step',
          },
        },
        {
          t: 'Block',
          key: 'log',
          title: [{ content: 'Output', alignment: 'Left' }, { content: [{ content: 'tail', style: { fg: 'dim' } }], alignment: 'Right' }],
          borders: 'ALL',
          borderType: 'Rounded',
          child: {
            t: 'Log',
            lines: [
              [{ content: '12:01:04 ', style: { fg: 'dim' } }, 'cargo test -p oino-ui'],
              [{ content: '12:01:09 ', style: { fg: 'dim' } }, { content: 'ok', style: { fg: 'success' } }, ' 207 layout vectors'],
              [{ content: '12:01:11 ', style: { fg: 'dim' } }, 'npm test -- src/ui'],
              [{ content: '12:01:15 ', style: { fg: 'dim' } }, { content: 'ok', style: { fg: 'success' } }, ' 137 goldens · 10 tree ops · 14 registry'],
              [{ content: '12:01:16 ', style: { fg: 'dim' } }, { content: 'warn', style: { fg: 'warning' } }, ' Select: interaction not asserted in jsdom'],
              [{ content: '12:01:18 ', style: { fg: 'dim' } }, 'vite --config vite.config.ui-capture.ts'],
            ],
            lineNumbers: true,
          },
        },
      ],
    },
    {
      t: 'KeyHints',
      key: 'keys',
      hints: [
        { keys: 'enter', label: 'open step' },
        { keys: 'ctrl+r', label: 'retry', action: 'retry' },
        { keys: 'esc', label: 'close' },
      ],
    },
  ],
};

/** Usage panel: tabs with local panes (chart, table, bars). */
export const USAGE: UiNode = {
  t: 'Block',
  title: 'Usage',
  borders: 'ALL',
  borderType: 'Rounded',
  child: {
    t: 'Tabs',
    titles: [
      { key: 'ctx', title: 'Context' },
      { key: 'cost', title: 'Cost', badge: [{ content: '$1.42', style: { fg: 'muted' } }] },
      { key: 'tools', title: 'Tools' },
    ],
    selected: 'ctx',
    onSelect: 'usage-tab',
    panes: {
      ctx: {
        t: 'Layout',
        key: 'ctx',
        constraints: [{ Length: 2 }, { Fill: 1 }],
        children: [
          { t: 'Gauge', ratio: 0.41, label: '82k / 200k tokens', gaugeStyle: { fg: 'info' } },
          {
            t: 'Chart',
            datasets: [
              { name: 'context', data: Array.from({ length: 40 }, (_, i) => [i, 10 + i * 1.8 + Math.sin(i / 3) * 6] as [number, number]), graphType: 'Line', style: { fg: 'info' } },
              { name: 'compaction', data: [[12, 32], [27, 58]], graphType: 'Scatter', style: { fg: 'harness' } },
            ],
            xAxis: { bounds: [0, 40], labels: ['turn 0', '20', '40'] },
            yAxis: { bounds: [0, 100], labels: ['0', '50k', '100k'] },
            legendPosition: 'TopRight',
          },
        ],
      },
      cost: {
        t: 'Table',
        key: 'cost',
        header: { cells: ['Model', 'In', 'Out', 'Cost'] },
        rows: [
          { key: 'a', cells: ['claude-opus', '1.2M', '84k', '$1.10'] },
          { key: 'b', cells: ['glm-5.3', '640k', '22k', '$0.21'] },
          { key: 'c', cells: ['deepseek-flash', '410k', '18k', '$0.11'] },
        ],
        footer: { cells: [{ content: 'Total', style: { addModifier: ['BOLD'] } }, '2.2M', '124k', { content: '$1.42', style: { fg: 'accent', addModifier: ['BOLD'] } }] },
        widths: [{ Fill: 2 }, { Length: 8 }, { Length: 8 }, { Length: 8 }],
        state: { selected: 'a' },
        onSelect: 'pick-model',
      },
      tools: {
        t: 'BarChart',
        key: 'tools',
        direction: 'Horizontal',
        data: [
          {
            bars: [
              { value: 42, label: 'read' },
              { value: 31, label: 'bash', style: { fg: 'c3' } },
              { value: 18, label: 'edit', style: { fg: 'c2' } },
              { value: 6, label: 'web', style: { fg: 'c4' } },
            ],
          },
        ],
        barStyle: { fg: 'c1' },
      },
    },
  },
};

/** A form-style panel (values collected on submit). */
export const FORM: UiNode = {
  t: 'Block',
  title: 'New cron job',
  borders: 'ALL',
  borderType: 'Rounded',
  padding: { left: 1, right: 1, top: 0, bottom: 0 },
  child: {
    t: 'Layout',
    spacing: 1,
    children: [
      { t: 'Input', key: 'name', label: 'Name', placeholder: 'nightly-report', onSubmit: 'create' },
      { t: 'Select', key: 'every', options: [{ value: 'h', label: 'Every hour' }, { value: 'd', label: 'Every day', description: 'at 03:00' }, { value: 'w', label: 'Every week' }], value: 'd' },
      { t: 'Toggle', key: 'notify', label: 'Notify me when it finishes', value: true },
      {
        t: 'Layout',
        direction: 'Horizontal',
        flex: 'End',
        spacing: 1,
        collapse: 0,
        constraints: [{ Length: 12 }, { Length: 14 }],
        children: [
          { t: 'Button', label: 'Cancel', action: 'cancel', variant: 'Ghost' },
          { t: 'Button', label: 'Create', action: 'create', variant: 'Primary', icon: 'plus' },
        ],
      },
    ],
  },
};

/** A tree of ~`n` nodes for the first-render perf row. */
export function bigPanel(n = 1000): UiNode {
  const children: UiNode[] = [];
  let count = 1;
  for (let i = 0; count < n; i++) {
    const items = Array.from({ length: 8 }, (_, j) => ({ key: `i${i}-${j}`, content: `Task ${i}.${j}`, detail: 'open', icon: 'dot' as const }));
    children.push({
      t: 'Block',
      key: `b${i}`,
      title: `Group ${i}`,
      borders: 'ALL',
      child: { t: 'Layout', children: [{ t: 'Gauge', key: 'g', ratio: (i % 10) / 10 }, { t: 'List', key: 'l', items }, { t: 'Paragraph', key: 'p', text: `Summary for group ${i}` }] },
    });
    count += 5 + items.length;
  }
  const columns: UiNode[] = [];
  for (let i = 0; i < children.length; i += 16) columns.push({ t: 'Layout', key: `c${i}`, children: children.slice(i, i + 16) });
  return { t: 'Layout', children: columns };
}
