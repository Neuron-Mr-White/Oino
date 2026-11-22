/**
 * Perf bench (ui-protocol §13.5, §13.7): the budget rows that run in jsdom.
 * The frame-rate rows (log append, 5 000-row scroll) and the heap row need a
 * real compositor — they run on the gallery perf page
 * (`vite.config.ui-capture.ts`, `?page=perf`) and on the capture device.
 *
 * vitest 5 removed the `bench()` export, so this samples the same rows the
 * perf page samples (setup outside the clock, `offsetHeight` flush) and runs
 * as a gated test: it prints the §13.5 table and fails on a 50× regression —
 * jsdom's DOM is ~5–15× slower than Chromium's (measured: the perf page
 * passes §13.5 while jsdom renders the same tree at ~12×), and the dev box is
 * shared (parallel workers double medians), so the gate ceiling is 50× budget;
 * the real §13.5 check runs on the perf page (desktop Chromium) and on the
 * Pixel-class device.
 */
import { afterAll, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { EntryView } from '../NodeView';
import { clearSolveCache, solveAxis } from '../layout/solve';
import { createUiRegistry } from '../registry';
import type { NodeOp, UiEntry, UiNode, UiPatch, UiSnapshot } from '../types';
import { bigPanel } from '../gallery/demos';

const target = { scope: 'session' as const, hostId: 'bench', sessionId: 'bench' };
const host = document.createElement('div');
document.body.append(host);

function entryOf(id: string, root: UiNode): UiEntry {
  return { id, extensionId: 'bench', scope: 'session', slot: 'sidePane', order: 0, seq: 1, view: { kind: 'tree', root }, rev: 1 };
}

const snapshotOf = (entries: UiEntry[], rev = 1): UiSnapshot => ({ scope: 'session', rev, entries, dialogs: [] });

/** A 1 000-node panel mounted through EntryView (auto mode), like a chat card. */
function mountBig(root: UiNode) {
  const reg = createUiRegistry('session');
  reg.applySnapshot(snapshotOf([entryOf('big', root)]));
  const dispose = render(() => <EntryView entry={reg.entry('big')!} target={target} mode="auto" />, host);
  return { reg, dispose };
}

/** 8 samples after 1 warmup, median (jsdom is noisy; the median is the row). */
function sample(fn: () => void): number {
  fn(); // warmup
  const t: number[] = [];
  for (let i = 0; i < 8; i++) {
    const t0 = performance.now();
    fn();
    t.push(performance.now() - t0);
  }
  t.sort((a, b) => a - b);
  return (t[3] + t[4]) / 2;
}

/** Snapshot-applied registries waiting to be mounted: setup is outside the clock, like the perf page. */
function preparedBig() {
  const reg = createUiRegistry('session');
  reg.applySnapshot(snapshotOf([entryOf('big', bigPanel(1000))]));
  return reg;
}
const rack = Array.from({ length: 9 }, () => preparedBig());
let rackNext = 0;

/** Keeps one big panel mounted across the patch row. */
const bigRoot = bigPanel(1000);
const big = mountBig(bigRoot);
let rev = 1;
const patch100 = (): UiPatch => {
  const set: NodeOp[] = [];
  for (let i = 0; i < 100; i++) {
    const col = Math.floor((i % 77) / 16) * 16;
    set.push({ op: 'set', path: [`c${col}`, `b${i % 77}`], props: { title: `Group ${i} · ${rev}` } });
  }
  return { scope: 'session', rev: ++rev, ops: [{ op: 'tree', id: 'big', rev, ops: set }] };
};

afterAll(() => {
  big.dispose();
  big.reg.dispose();
  rack.forEach((r) => r.dispose());
  host.remove();
});

describe('§13.5 perf rows (jsdom)', () => {
  it(
    'samples the four jsdom rows, fails only on a 50× regression',
    () => {
      const budgets: Array<[string, number, () => void]> = [
        [
          'first render, 1 000-node panel',
          16,
          () => {
            const reg = rack[rackNext++ % rack.length]!;
            const dispose = render(() => <EntryView entry={reg.entry('big')!} target={target} mode="auto" />, host);
            void host.offsetHeight;
            dispose();
          },
        ],
        [
          'apply a 100-op patch',
          2,
          () => {
            big.reg.applyPatch(patch100());
            void host.offsetHeight;
          },
        ],
        [
          'registry snapshot apply (64 entries)',
          5,
          () => {
            const reg = createUiRegistry('session');
            const entries = Array.from({ length: 64 }, (_, i) =>
              entryOf(`e${i}`, { t: 'List', items: Array.from({ length: 12 }, (_, j) => ({ key: `k${j}`, content: `Item ${j}` })) }),
            );
            reg.applySnapshot(snapshotOf(entries));
            reg.dispose();
          },
        ],
        [
          'layout solve (16 children, uncached)',
          0.2,
          () => {
            clearSolveCache();
            solveAxis(
              {
                constraints: Array.from({ length: 16 }, (_, i) => [{ Length: 3 }, { Fill: 1 }, { Percentage: 10 }, { Min: 2 }, { Max: 8 }][i % 5]!),
                spacing: 1,
              },
              200,
            );
          },
        ],
        [
          'layout solve (16 children, L/P/F fast path)',
          0.2,
          () => {
            clearSolveCache();
            solveAxis(
              {
                constraints: Array.from({ length: 16 }, (_, i) => [{ Length: 3 }, { Fill: 2 }, { Percentage: 10 }][i % 3]!),
                spacing: 1,
              },
              200,
            );
          },
        ],
      ];

      const lines: string[] = [];
      for (const [name, budget, fn] of budgets) {
        const ms = sample(fn);
        lines.push(`${name.padEnd(38)} median ${ms.toFixed(3)} ms   §13.5 desktop ≤ ${budget} ms${ms > budget * 50 ? '   ✗ >50× regression' : ''}`);
        expect(ms, `${name} regressed past 50× budget`).toBeLessThan(budget * 50);
      }
      console.info(`[ui §13.5 jsdom]\n${lines.join('\n')}`);
    },
    120_000,
  );
});
