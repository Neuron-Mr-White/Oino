/** UNI-222: long-horizon progress — mermaid builder, formatters, store wiring. */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { agoText, clockText, isProgress, mermaidText, nodeId, progressMermaid, runPercent, runSegments, runSettled, shownRun, type LhProgress, type LhRun } from './lhProgress';
import { renderMermaid, resetMermaidCache } from '../../lib/markdown/mermaid';
import { createChat } from '../../lib/chat/store';
import { setChatTransport, type ChatTransport } from '../../lib/session/link';

if (!(SVGElement.prototype as unknown as { getBBox?: unknown }).getBBox) {
  (SVGElement.prototype as unknown as { getBBox: () => DOMRect }).getBBox = () => ({ x: 0, y: 0, width: 10, height: 10 }) as DOMRect;
}

export const graphRun: LhRun = {
  mode: 'graph',
  title: 'Repo health',
  status: 'running',
  items: [
    { id: 'scan', label: 'Scan the repo for "TODO" <markers>', status: 'done', deps: [], wave: 0, summary: '12 found' },
    { id: 'lint-1', label: 'Lint', status: 'running', deps: [], wave: 0 },
    { id: 'report', label: 'Write [report]', status: 'queued', deps: ['scan', 'lint-1'], wave: 1 },
  ],
  counts: { total: 3, done: 1, running: 1, failed: 0, queued: 1 },
};

describe('progressMermaid', () => {
  it('graph: waves are subgraphs, deps are edges, nodes carry their status class', () => {
    const src = progressMermaid(graphRun);
    expect(src.split('\n')[0]).toBe('flowchart LR');
    expect(src).toContain('subgraph wave0["Wave 1 · 1/2"]');
    expect(src).toContain('subgraph wave1["Wave 2 · 0/1"]');
    expect(src).toContain(`${nodeId('scan', 0)} --> ${nodeId('report', 2)}`);
    expect(src).toContain(`${nodeId('lint-1', 1)} --> ${nodeId('report', 2)}`);
    expect(src).toMatch(/n0_scan\[".*"\]:::done/);
    expect(src).toMatch(/n1_lint_1\[".*"\]:::running/);
    expect(src).toContain('classDef running');
    // Untrusted labels can't break out of the quoted label.
    expect(src).not.toMatch(/"TODO"|<markers>|\[report\]/);
  });

  it('swarm: one subgraph, no edges', () => {
    const src = progressMermaid({ ...graphRun, mode: 'swarm', items: graphRun.items.map((i) => ({ ...i, deps: [], wave: undefined })) });
    expect(src).toContain('subgraph items["Repo health · 1/3"]');
    expect(src).not.toContain('-->');
    expect(src).toContain(`${nodeId('scan', 0)} ~~~ ${nodeId('lint-1', 1)} ~~~ ${nodeId('report', 2)}`);
  });

  it('renders through the app mermaid renderer (valid source)', async () => {
    resetMermaidCache();
    const res = await renderMermaid(progressMermaid(graphRun));
    expect(res.ok).toBe(true);
  });

  it('mermaidText flattens and clips', () => {
    expect(mermaidText('a\n  b "c" [d]|e#f;')).toBe("a b 'c' d e f");
    expect(mermaidText('x'.repeat(100), 10)).toHaveLength(10);
  });
});

describe('helpers', () => {
  it('shownRun prefers current, else last (not live)', () => {
    const p: LhProgress = { v: 1, mode: 'none', last: { ...graphRun, status: 'done' }, log: [], updatedAt: 0 };
    expect(shownRun(p)).toEqual({ run: p.last, live: false });
    expect(shownRun({ ...p, mode: 'graph', current: graphRun })).toEqual({ run: graphRun, live: true });
    expect(shownRun(undefined)).toBeUndefined();
  });
  it('runPercent per mode', () => {
    expect(runPercent(graphRun)).toBe(33);
    expect(runPercent({ ...graphRun, mode: 'goal', goal: { objective: 'x', status: 'active', turn: 1, maxTurns: 9, percent: 40 } })).toBe(40);
    expect(runPercent({ ...graphRun, mode: 'ralph', ralph: { name: 'r', iteration: 1, maxIterations: 3, checked: 1, total: 4 } })).toBe(25);
    expect(runPercent({ ...graphRun, counts: { total: 0, done: 0, running: 0, failed: 0, queued: 0 } })).toBe(0);
  });
  it('runSegments: done · failed · running; settled counts done + failed (1 each of 4 → 2/4)', () => {
    const c = { total: 4, done: 1, failed: 1, running: 1, queued: 1 };
    expect(runSegments(c).map((s) => [s.tone, s.pct])).toEqual([['done', 25], ['failed', 25], ['running', 25]]);
    expect(runSettled({ ...graphRun, counts: c })).toBe(2);
    expect(runPercent({ ...graphRun, counts: c })).toBe(50);
    expect(runSegments({ total: 0, done: 0, failed: 0, running: 0, queued: 0 })).toEqual([]);
  });
  it('agoText / clockText / isProgress', () => {
    expect(agoText(4000)).toBe('4s ago');
    expect(agoText(4 * 60_000)).toBe('4m ago');
    expect(clockText(new Date(2026, 0, 1, 9, 5, 7).getTime())).toBe('09:05:07');
    expect(isProgress({ v: 1, log: [] })).toBe(true);
    expect(isProgress({ v: 2, log: [] })).toBe(false);
    expect(isProgress(null)).toBe(false);
  });
});

describe('chat store — lh_progress (UNI-222)', () => {
  let sent: Array<Record<string, unknown>>;
  let push: (m: Record<string, unknown>) => void;
  beforeEach(() => {
    sent = [];
    const transport: ChatTransport = {
      list: async () => ({ sessions: [] }),
      open: (_h, _p, onLine, onState) => {
        push = (m) => onLine(JSON.stringify(m));
        queueMicrotask(() => onState('open'));
        return { send: async (line) => void sent.push(JSON.parse(line)), close: () => {} };
      },
    };
    setChatTransport(transport);
  });
  afterEach(() => setChatTransport(undefined));

  const hello = (extra: Record<string, unknown> = {}) => ({ t: 'hello', session: { id: 's', cwd: '/p' }, state: { running: false }, entries: [], commands: [], models: [], dialogs: [], queue: [], ...extra });
  const progress: LhProgress = { v: 1, mode: 'graph', current: graphRun, log: [{ at: 1, text: 'scan done', item: 'scan', status: 'done' }], updatedAt: 1 };

  it('hello.lhProgress seeds the state; pushes replace it; garbage is ignored', async () => {
    await new Promise<void>((resolve) =>
      createRoot((dispose) => {
        const chat = createChat('h', 's1');
        void (async () => {
          await Promise.resolve();
          push(hello({ lhProgress: progress }));
          expect(chat.state.lhProgress?.current?.title).toBe('Repo health');
          push({ t: 'lh_progress', progress: { ...progress, mode: 'none', current: undefined, last: graphRun } });
          expect(chat.state.lhProgress?.current).toBeUndefined();
          expect(chat.state.lhProgress?.last?.title).toBe('Repo health');
          expect(chat.state.lhProgressSupported).toBe(true);
          push({ t: 'lh_progress', progress: { nope: 1 } });
          expect(chat.state.lhProgress).toBeUndefined();
          push(hello());
          expect(chat.state.lhProgress).toBeUndefined();
          dispose();
          resolve();
        })();
      }),
    );
  });

  it('refreshProgress sends lh_progress_get; an older bridge error marks it unsupported without a toast', async () => {
    await new Promise<void>((resolve) =>
      createRoot((dispose) => {
        const chat = createChat('h', 's1');
        void (async () => {
          await new Promise((r) => setTimeout(r, 0));
          push(hello());
          chat.refreshProgress();
          const req = sent.find((m) => m.t === 'lh_progress_get');
          expect(req?.ref).toBeTruthy();
          push({ t: 'error', message: 'unknown type lh_progress_get', ref: req!.ref });
          expect(chat.state.lhProgressSupported).toBe(false);
          expect(chat.state.error).toBeUndefined();
          dispose();
          resolve();
        })();
      }),
    );
  });
});
