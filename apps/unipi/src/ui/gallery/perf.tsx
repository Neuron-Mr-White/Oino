/**
 * Perf page (ui-protocol §13.5): the budget rows measured in a real browser
 * (desktop Chromium, and the capture page on the Android device). Results go
 * to `window.__UI_PERF__` for scripted capture.
 */
import { For, Show, createSignal, onMount } from 'solid-js';
import { render } from 'solid-js/web';
import { EntryView } from '../NodeView';
import { clearSolveCache, solveAxis } from '../layout/solve';
import { createUiRegistry } from '../registry';
import type { NodeOp, RegistryOp, UiEntry, UiNode } from '../types';
import type { AxisSpec } from '../layout/solve';
import { bigPanel } from './demos';

type Row = { name: string; value: number; unit: string; desktop: number; phone: number; lowerIsBetter?: boolean };

const target = { scope: 'session' as const, hostId: 'perf', sessionId: 'perf' };
const nextFrame = () => new Promise<number>((r) => requestAnimationFrame(r));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function countNodes(n: unknown): number {
  if (!n || typeof n !== 'object') return 0;
  const o = n as Record<string, unknown>;
  let c = typeof o.t === 'string' ? 1 : 0;
  for (const k of ['children', 'items', 'rows', 'nodes', 'lines']) if (Array.isArray(o[k])) for (const x of o[k] as unknown[]) c += typeof x === 'object' && x && 't' in x ? countNodes(x) : 1;
  if (o.child) c += countNodes(o.child);
  return c;
}

function entryOf(id: string, root: UiNode, slot: UiEntry['slot'] = 'sidePane'): UiEntry {
  return { id, extensionId: 'perf', scope: 'session', slot, order: 0, seq: 1, view: { kind: 'tree', root }, rev: 1 };
}

/** Mounts `entry` from `reg` into `host` and returns the disposer. */
function mount(host: HTMLElement, reg: ReturnType<typeof createUiRegistry>, id: string, definite = false) {
  return render(() => <EntryView entry={reg.entry(id)!} target={target} mode={definite ? 'definite' : 'auto'} />, host);
}

async function fps(seconds: number, step?: (t: number) => void): Promise<{ fps: number; longTasks: number }> {
  let long = 0;
  const po = typeof PerformanceObserver !== 'undefined' && PerformanceObserver.supportedEntryTypes?.includes('longtask') ? new PerformanceObserver((l) => (long += l.getEntries().length)) : undefined;
  po?.observe({ entryTypes: ['longtask'] });
  const t0 = await nextFrame();
  let frames = 0;
  let now = t0;
  while (now - t0 < seconds * 1000) {
    step?.(now - t0);
    now = await nextFrame();
    frames++;
  }
  po?.disconnect();
  return { fps: (frames * 1000) / (now - t0), longTasks: long };
}

export async function runPerf(stage: HTMLElement): Promise<Row[]> {
  const rows: Row[] = [];
  // 1 — first render of a 1 000-node panel.
  {
    const root = bigPanel(1000);
    const reg = createUiRegistry('session');
    reg.applySnapshot({ scope: 'session', rev: 1, entries: [entryOf('big', root)], dialogs: [] });
    await nextFrame();
    const t = performance.now();
    const dispose = mount(stage, reg, 'big');
    void stage.offsetHeight;
    const ms = performance.now() - t;
    rows.push({ name: `First render, ${countNodes(root)}-node panel`, value: ms, unit: 'ms', desktop: 16, phone: 50 });
    // 2 — apply a 100-op patch while mounted (Solid updates synchronously).
    const real: NodeOp[] = [];
    for (let i = 0; i < 100; i++) {
      const col = Math.floor((i % 77) / 16) * 16;
      real.push({ op: 'set', path: [`c${col}`, `b${i % 77}`], props: { title: `Group ${i} · updated` } });
    }
    const t2 = performance.now();
    const out = reg.applyPatch({ scope: 'session', rev: 2, ops: [{ op: 'tree', id: 'big', rev: 2, ops: real }] });
    void stage.offsetHeight;
    rows.push({ name: `Apply a 100-op patch${out.ok ? '' : ` (failed: ${out.error})`}`, value: performance.now() - t2, unit: 'ms', desktop: 2, phone: 6 });
    dispose();
    reg.dispose();
  }
  await nextFrame();
  // 3 — registry snapshot apply (64 entries).
  {
    const reg = createUiRegistry('session');
    const entries = Array.from({ length: 64 }, (_, i) => entryOf(`e${i}`, { t: 'List', items: Array.from({ length: 12 }, (_, j) => ({ key: `k${j}`, content: `Item ${j}` })) }, 'widget'));
    const t = performance.now();
    reg.applySnapshot({ scope: 'session', rev: 1, entries, dialogs: [] });
    rows.push({ name: 'Registry snapshot apply (64 entries)', value: performance.now() - t, unit: 'ms', desktop: 5, phone: 15 });
    reg.dispose();
  }
  // 4 — layout solve (16 children, uncached): the L/P/F closed form and the Min/Max kiwi path.
  {
    const shapes = [
      ['Layout solve (16 children, L/P/F)', Array.from({ length: 16 }, (_, i) => [{ Length: 3 }, { Fill: 2 }, { Percentage: 10 }][i % 3]!)],
      ['Layout solve (16 children, Min/Max)', Array.from({ length: 16 }, (_, i) => [{ Length: 3 }, { Fill: 1 }, { Percentage: 10 }, { Min: 2 }, { Max: 8 }][i % 5]!)],
    ] as const;
    for (const [name, cs] of shapes) {
      const n = 200;
      const t = performance.now();
      for (let i = 0; i < n; i++) {
        clearSolveCache();
        solveAxis({ constraints: cs as unknown as AxisSpec['constraints'], spacing: 1 }, 200 + i);
      }
      rows.push({ name, value: (performance.now() - t) / n, unit: 'ms', desktop: 0.2, phone: 0.6 });
    }
  }
  // 5 — Log append, 200 lines/s sustained with follow.
  {
    const reg = createUiRegistry('session');
    reg.applySnapshot({ scope: 'session', rev: 1, entries: [entryOf('log', { t: 'Log', lines: [], maxLines: 5000 })], dialogs: [] });
    const dispose = mount(stage, reg, 'log', true);
    let rev = 1;
    let sent = 0;
    let lastAt = 0;
    const r = await fps(3, (t) => {
      // 10 lines every 50 ms = 200 lines/s.
      if (t - lastAt < 50) return;
      lastAt = t;
      const items: RegistryOp[] = [
        { op: 'tree', id: 'log', rev: ++rev, ops: [{ op: 'append', path: [], field: 'lines', items: Array.from({ length: 10 }, () => `${String(sent++).padStart(6, '0')} build step ok · ${Math.random().toString(36).slice(2, 10)}`) }] },
      ];
      reg.applyPatch({ scope: 'session', rev, ops: items });
    });
    rows.push({ name: `Log append 200 lines/s + follow (${r.longTasks} long tasks)`, value: r.fps, unit: 'fps', desktop: 60, phone: 50, lowerIsBetter: false });
    dispose();
    reg.dispose();
  }
  // 6 — scroll 5 000-row List and Table.
  for (const kind of ['List', 'Table'] as const) {
    const reg = createUiRegistry('session');
    const root: UiNode =
      kind === 'List'
        ? { t: 'List', items: Array.from({ length: 5000 }, (_, i) => ({ key: `r${i}`, content: `Row ${i}`, detail: i % 3 ? undefined : 'detail line' })) }
        : { t: 'Table', header: { cells: ['#', 'Name', 'State'] }, widths: [{ Length: 6 }, { Fill: 1 }, { Length: 10 }], rows: Array.from({ length: 5000 }, (_, i) => ({ key: `r${i}`, cells: [String(i), `Task number ${i}`, i % 2 ? 'open' : 'done'] })) };
    reg.applySnapshot({ scope: 'session', rev: 1, entries: [entryOf(kind, root)], dialogs: [] });
    const dispose = mount(stage, reg, kind, true);
    await nextFrame();
    const scroller = stage.querySelector<HTMLElement>(kind === 'List' ? '.ui-list' : '.ui-table-scroll')!;
    const r = await fps(2, (t) => {
      scroller.scrollTop = (t / 2000) * (scroller.scrollHeight - scroller.clientHeight);
    });
    rows.push({ name: `Scroll a 5 000-row ${kind} (${r.longTasks} long tasks)`, value: r.fps, unit: 'fps', desktop: 60, phone: 50, lowerIsBetter: false });
    dispose();
    reg.dispose();
    await sleep(50);
  }
  // 7 — memory per open 5 000-line Log (Chromium only).
  {
    const mem = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
    if (mem) {
      const gc = (window as unknown as { gc?: () => void }).gc;
      gc?.();
      const before = mem.usedJSHeapSize;
      const reg = createUiRegistry('session');
      reg.applySnapshot({ scope: 'session', rev: 1, entries: [entryOf('mem', { t: 'Log', maxLines: 5000, lines: Array.from({ length: 5000 }, (_, i) => `${i} the quick brown fox jumps over the lazy dog ${i}`) })], dialogs: [] });
      const dispose = mount(stage, reg, 'mem', true);
      await nextFrame();
      gc?.();
      const mb = (mem.usedJSHeapSize - before) / 1048576;
      rows.push({ name: `Memory, open 5 000-line Log${gc ? '' : ' (no gc(): upper bound)'}`, value: mb, unit: 'MB', desktop: 4, phone: 4 });
      dispose();
      reg.dispose();
    }
  }
  return rows;
}

export function PerfPage() {
  const [rows, setRows] = createSignal<Row[]>([]);
  const [running, setRunning] = createSignal(true);
  let stage!: HTMLDivElement;
  const phone = document.documentElement.dataset.shell !== 'desktop';
  onMount(async () => {
    await sleep(300);
    const r = await runPerf(stage);
    setRows(r);
    setRunning(false);
    (window as unknown as { __UI_PERF__: Row[] }).__UI_PERF__ = r;
  });
  const ok = (r: Row) => {
    const budget = phone ? r.phone : r.desktop;
    return r.lowerIsBetter === false ? r.value >= budget * 0.97 : r.value <= budget;
  };
  return (
    <section class="g-section">
      <h2 class="g-kind">Performance budget (§13.5) · {phone ? 'phone' : 'desktop'} column</h2>
      <Show when={!running()} fallback={<p class="g-meta">Measuring…</p>}>
        <table class="g-perf">
          <thead>
            <tr>
              <th>Metric</th>
              <th>Measured</th>
              <th>Budget</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(r) => (
                <tr>
                  <td>{r.name}</td>
                  <td class="num">
                    {r.value.toFixed(r.unit === 'ms' && r.value < 1 ? 3 : 1)} {r.unit}
                  </td>
                  <td class="num">
                    {r.lowerIsBetter === false ? '≥' : '≤'} {phone ? r.phone : r.desktop} {r.unit}
                  </td>
                  <td class={ok(r) ? 'g-ok' : 'g-bad'}>{ok(r) ? 'pass' : 'over'}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
      <div ref={stage} class="g-stage" />
    </section>
  );
}
