// @vitest-environment jsdom
/**
 * UNI-222: the Progress sheet (empty / unsupported / graph+swarm chart /
 * ralph checklist / goal bars / last run / log) and the header ⋯ "Progress" row.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { ProgressSheet, progressActive } from './ProgressSheet';
import { ChatHeaderMenu } from './SessionMenus';
import type { Chat } from '../../lib/chat/store';
import type { LhProgress, LhRun } from '../status/lhProgress';

vi.mock('../../lib/markdown/mermaid', () => ({ renderMermaid: async (src: string) => ({ ok: true, svg: `<svg data-src="${src.length}"><g class="node"></g></svg>` }) }));
const opened: Array<{ svg: string; source: string }> = [];
vi.mock('./MermaidViewer', () => ({ openMermaidViewer: (r: { svg: string; source: string }) => opened.push(r) }));

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

function fakeChat(extra: Record<string, unknown> = {}, api: Record<string, unknown> = {}) {
  const [state, setState] = createStore<Record<string, unknown>>({ conn: 'live', run: { running: false }, work: [], ...extra });
  const refreshProgress = vi.fn();
  const graphHistory = vi.fn(async (): Promise<LhRun[] | { error: string }> => []);
  const graphOutput = vi.fn(async (): Promise<unknown> => ({ error: 'nope' }));
  const workTranscript = vi.fn(async (): Promise<unknown> => ({ error: 'nope' }));
  const graphStop = vi.fn(async () => {});
  const chat = { state, refreshProgress, graphHistory, graphOutput, workTranscript, graphStop, ...api } as unknown as Chat;
  return { chat, setState, refreshProgress, graphHistory, graphOutput, workTranscript, graphStop, api };
}

const graph: LhRun = {
  mode: 'graph',
  title: 'Repo health',
  status: 'running',
  items: [
    { id: 'scan', label: 'Scan the repo', status: 'done', deps: [], wave: 0, summary: '12 TODOs' },
    { id: 'lint', label: 'Lint', status: 'running', deps: [], wave: 0 },
    { id: 'report', label: 'Write the report', status: 'queued', deps: ['scan', 'lint'], wave: 1 },
  ],
  counts: { total: 3, done: 1, running: 1, failed: 0, queued: 1 },
};
const live = (run: LhRun): LhProgress => ({ v: 1, mode: run.mode, current: run, log: [{ at: 1, text: 'scan done', item: 'scan', status: 'done' }, { at: 2, text: 'lint running', item: 'lint', status: 'running' }], updatedAt: Date.now() });

describe('ProgressSheet', () => {
  it('empty: EmptyState with how to start a run; asks the bridge once on open', () => {
    const f = fakeChat();
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-empty').textContent).toContain('No long-horizon run');
    expect(screen.getByTestId('progress-empty').textContent).toContain('/unipi:graph');
    expect(f.refreshProgress).toHaveBeenCalledTimes(1);
  });

  it('older bridge: says to update unipi', () => {
    const f = fakeChat({ lhProgressSupported: false });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-empty').textContent).toContain('needs a newer unipi');
  });

  it('graph: chart, counts, item list with detail, newest-first log; live-updates', async () => {
    const f = fakeChat({ lhProgress: live(graph) });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByText('Graph progress')).toBeTruthy();
    expect(screen.getByText('1/3')).toBeTruthy();
    const chart = await waitFor(() => screen.getByTestId('progress-chart'));
    expect(chart.innerHTML).toContain('<svg');
    fireEvent.click(chart);
    expect(opened.at(-1)?.source).toContain('flowchart LR');
    const items = screen.getByTestId('progress-items');
    expect(items.querySelectorAll('li')).toHaveLength(3);
    expect(items.querySelector('li[data-status="running"] .progress-spinner')).toBeTruthy();
    fireEvent.click(screen.getByText('report'));
    expect(screen.getByText(/Wave 2 · needs scan, lint/)).toBeTruthy();
    const log = screen.getByTestId('progress-log').querySelectorAll('li');
    expect(log[0]!.textContent).toContain('lint running');
    expect(log[0]!.hasAttribute('data-newest')).toBe(true);
    // Live update: report starts.
    f.setState('lhProgress', live({ ...graph, items: graph.items.map((i) => (i.id === 'lint' ? { ...i, status: 'done' } : i.id === 'report' ? { ...i, status: 'running' } : i)), counts: { total: 3, done: 2, running: 1, failed: 0, queued: 0 } }));
    await waitFor(() => expect(screen.getByText('2/3')).toBeTruthy());
  });

  it('swarm with no items yet: waiting note, no chart', () => {
    const f = fakeChat({ lhProgress: live({ ...graph, mode: 'swarm', items: [], counts: { total: 0, done: 0, running: 0, failed: 0, queued: 0 } }) });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByText(/Waiting for the plan/)).toBeTruthy();
    expect(screen.queryByTestId('progress-chart')).toBeNull();
  });

  it('ralph: iteration + checklist with this-iteration rows', () => {
    const f = fakeChat({
      lhProgress: live({
        mode: 'ralph',
        title: 'docs',
        status: 'running',
        items: [
          { id: '1', label: 'Write intro', status: 'done', deps: [] },
          { id: '2', label: 'Fix links', status: 'running', deps: [] },
          { id: '3', label: 'Publish', status: 'queued', deps: [] },
        ],
        counts: { total: 3, done: 1, running: 1, failed: 0, queued: 1 },
        ralph: { name: 'docs', iteration: 2, maxIterations: 10, checked: 1, total: 3 },
      }),
    });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByText(/iteration 2\s*of 10/)).toBeTruthy();
    expect(screen.getByText('1 of 3 checked · 33%')).toBeTruthy();
    const rows = screen.getByTestId('ralph-checklist').querySelectorAll('li');
    expect(rows).toHaveLength(3);
    expect(rows[1]!.textContent).toContain('this iteration');
  });

  it('goal: % bar, turns, latest estimate', () => {
    const f = fakeChat({
      lhProgress: live({
        mode: 'goal',
        title: 'Ship the parser',
        status: 'running',
        items: [],
        counts: { total: 0, done: 0, running: 0, failed: 0, queued: 0 },
        goal: { objective: 'Ship the parser', status: 'active', turn: 4, maxTurns: 30, percent: 55, summary: 'Parser done; tests left.', estimatedAt: Date.now() - 30_000 },
      }),
    });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('goal-percent').textContent).toBe('~55%');
    expect(screen.getByTestId('goal-bar').getAttribute('aria-valuenow')).toBe('55');
    expect(screen.getByText(/turn 4 of 30/)).toBeTruthy();
    expect(screen.getByText(/Parser done; tests left\./)).toBeTruthy();
    expect(screen.getByText(/· 30s ago/)).toBeTruthy();
  });

  it('goal mode on but no run yet: says the mode is on (UNI-274)', () => {
    const f = fakeChat({ lhMode: { mode: 'goal' }, lhProgress: { v: 1, mode: 'none', log: [], updatedAt: Date.now() } });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-empty').textContent).toContain('Goal mode is on');
  });

  it('log shows the newest 3, expandable', () => {
    const lines = Array.from({ length: 6 }, (_, i) => ({ at: i, text: `step ${i}` }));
    const f = fakeChat({ lhProgress: { ...live(graph), log: lines } });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-log').querySelectorAll('li')).toHaveLength(3);
    fireEvent.click(screen.getByText('Show all 6'));
    expect(screen.getByTestId('progress-log').querySelectorAll('li')).toHaveLength(6);
  });

  it('idle with a last run: says no mode active and shows the finished run', () => {
    const f = fakeChat({ lhProgress: { v: 1, mode: 'none', last: { ...graph, status: 'failed', reason: 'settled(with_failures)', endedAt: Date.now() - 120_000 }, log: [], updatedAt: Date.now() } });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-idle').textContent).toContain('No long-horizon mode active · last run ended 2m ago');
    expect(screen.getByText('Graph · Finished with failures')).toBeTruthy();
    expect(screen.getByText('settled(with_failures)')).toBeTruthy();
  });

  it('progressActive only while a run is running', () => {
    expect(progressActive(fakeChat({ lhProgress: live(graph) }).chat)).toBe(true);
    expect(progressActive(fakeChat({ lhProgress: live({ ...graph, status: 'paused' }) }).chat)).toBe(false);
    expect(progressActive(undefined)).toBe(false);
  });
});

describe('ProgressSheet agent graph detail (UNI-282)', () => {
  const t0 = Date.now() - 200_000;
  const detailed: LhRun = {
    ...graph,
    graphId: 'g-1',
    selected: ['r-scan'],
    items: [
      { id: 'scan', label: 'Scan the repo', status: 'done', deps: [], wave: 0, summary: '12 TODOs', operatorId: 'op-scan', resultId: 'r-scan', profile: 'worker', startedAt: t0, endedAt: t0 + 185_000, toolCalls: 12, tokens: 34_200 },
      { id: 'lint', label: 'Lint', status: 'running', deps: [], wave: 0, operatorId: 'op-lint', startedAt: Date.now() - 42_000, toolCalls: 3 },
      { id: 'report', label: 'Write the report', status: 'queued', deps: ['scan', 'lint'], wave: 1 },
    ],
  };

  it('an expanded step shows profile, duration, tool calls, tokens and the summary; selected chip on chosen results', () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    const chips = screen.getAllByTestId('selected-chip');
    expect(chips).toHaveLength(1);
    expect(chips[0]!.closest('li')!.textContent).toContain('scan');
    fireEvent.click(screen.getByText('scan'));
    expect(screen.getByTestId('step-facts').textContent).toBe('worker · 3m 05s · 12 tool calls · 34k tokens');
    expect(screen.getByText('12 TODOs')).toBeTruthy();
    expect(screen.getByTestId('step-result')).toBeTruthy();
    expect(screen.getByTestId('step-session')).toBeTruthy();
    // A running step: live elapsed time; a queued one has nothing to open.
    fireEvent.click(screen.getByText('lint'));
    expect(screen.getByTestId('step-facts').textContent).toMatch(/^4[0-9]s · 3 tool calls$/);
    fireEvent.click(screen.getByText('report'));
    expect(screen.queryByTestId('step-result')).toBeNull();
    expect(screen.queryByTestId('step-session')).toBeNull();
  });

  it('Full result fetches lh_graph output and renders it as markdown', async () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    f.graphOutput.mockResolvedValue({ graphId: 'g-1', id: 'scan', status: 'completed', resultId: 'r-scan', instruction: 'Scan', text: '## Findings\n\n- **12** TODOs in `src/`' });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByText('scan'));
    fireEvent.click(screen.getByTestId('step-result'));
    expect(f.graphOutput).toHaveBeenCalledWith('scan', 'g-1');
    const panel = await waitFor(() => {
      const p = screen.getByTestId('step-result-panel');
      expect(p.querySelector('h2')?.textContent).toBe('Findings');
      return p;
    });
    expect(panel.querySelector('strong')?.textContent).toBe('12');
  });

  it('an open step and its result survive a live push', async () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    f.graphOutput.mockResolvedValue({ graphId: 'g-1', id: 'scan', status: 'completed', instruction: 'Scan', text: 'kept text' });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByText('scan'));
    fireEvent.click(screen.getByTestId('step-result'));
    await waitFor(() => expect(screen.getByTestId('step-result-panel').textContent).toContain('kept text'));
    f.setState('lhProgress', live({ ...detailed, items: detailed.items.map((i) => (i.id === 'lint' ? { ...i, toolCalls: 9 } : { ...i })) }));
    await Promise.resolve();
    expect(screen.getByTestId('step-result-panel').textContent).toContain('kept text');
    expect(f.graphOutput).toHaveBeenCalledTimes(1);
  });

  it('Full result on an older bridge says it needs a newer unipi', async () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    f.graphOutput.mockResolvedValue({ error: 'Reading a step’s result needs a newer unipi on the PC.', unsupported: true });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByText('scan'));
    fireEvent.click(screen.getByTestId('step-result'));
    await waitFor(() => expect(screen.getByTestId('step-result-panel').textContent).toContain('needs a newer unipi on the PC'));
  });

  it('Open session fetches the step subagent transcript: task, text, tool rows with collapsible output, report', async () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    f.workTranscript.mockResolvedValue({
      id: 'op-scan',
      items: [
        { kind: 'task', text: 'Scan the repo for TODOs' },
        { kind: 'text', text: 'Looking at **src**.' },
        { kind: 'tool', name: 'bash', arg: 'rg TODO src', output: 'src/a.ts:1: TODO', isError: false, running: false, durationMs: 1200 },
        { kind: 'tool', name: 'read', arg: 'missing.ts', output: 'ENOENT', isError: true, running: false },
      ],
      report: 'Found 12.',
    });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByText('scan'));
    fireEvent.click(screen.getByTestId('step-session'));
    expect(f.workTranscript).toHaveBeenCalledWith('op-scan');
    const tx = await waitFor(() => screen.getByTestId('step-transcript'));
    expect(tx.querySelector('[data-kind="task"]')!.textContent).toContain('Scan the repo for TODOs');
    expect(tx.querySelector('[data-kind="text"] strong')?.textContent).toBe('src');
    const tools = tx.querySelectorAll('[data-kind="tool"]');
    expect(tools).toHaveLength(2);
    expect(tools[0]!.textContent).toContain('bash');
    expect(tools[0]!.textContent).toContain('rg TODO src');
    expect(tools[0]!.textContent).toContain('1s');
    expect(tools[1]!.hasAttribute('data-error')).toBe(true);
    expect(screen.queryByTestId('tool-output')).toBeNull();
    fireEvent.click(tools[0]!.querySelector('button')!);
    expect(screen.getByTestId('tool-output').textContent).toBe('src/a.ts:1: TODO');
    expect(screen.getByTestId('step-session-panel').textContent).toContain('Found 12.');
  });

  it('Stop graph asks first, then sends lh_graph stop', async () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-settled').textContent).toBe('1 of 3 settled');
    fireEvent.click(screen.getByTestId('graph-stop'));
    expect(f.graphStop).not.toHaveBeenCalled();
    expect(screen.getByTestId('graph-stop-confirm').textContent).toContain('Stop the graph?');
    fireEvent.click(screen.getByTestId('graph-stop-go'));
    await waitFor(() => expect(f.graphStop).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByTestId('progress-flash').textContent).toContain('Stopping the graph'));
  });

  it('Stop graph: cancel sends nothing; not offered for a finished run', () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByTestId('graph-stop'));
    fireEvent.click(screen.getByText('Cancel'));
    expect(f.graphStop).not.toHaveBeenCalled();
    cleanup();
    const g = fakeChat({ lhProgress: { v: 1, mode: 'none', last: { ...detailed, status: 'done' }, log: [], updatedAt: Date.now() } });
    render(() => <ProgressSheet open onClose={() => {}} chat={g.chat} />);
    expect(screen.queryByTestId('graph-stop')).toBeNull();
  });

  it('run picker loads earlier graphs on open and shows a past run read-only; Full result uses its graphId', async () => {
    const past: LhRun = {
      mode: 'swarm',
      title: 'Audit deps',
      status: 'done',
      graphId: 'g-0',
      selected: ['r-a'],
      endedAt: Date.now() - 3_600_000,
      items: [
        { id: 'a', label: 'Check lodash', status: 'done', deps: [], resultId: 'r-a', operatorId: 'op-a', summary: 'ok' },
        { id: 'b', label: 'Check react', status: 'aborted', deps: [], summary: 'stopped(user_requested)' },
      ],
      counts: { total: 2, done: 1, running: 0, failed: 0, queued: 0 },
    };
    const f = fakeChat({ lhProgress: live(detailed) });
    f.graphHistory.mockResolvedValue([past]);
    f.graphOutput.mockResolvedValue({ graphId: 'g-0', id: 'a', status: 'completed', instruction: 'x', text: 'lodash fine' });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(f.graphHistory).toHaveBeenCalledTimes(1);
    const earlier = await waitFor(() => screen.getByTestId('run-earlier'));
    expect(earlier.textContent).toContain('Earlier runs · 1');
    expect(screen.getByTestId('run-current').getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(earlier);
    const list = screen.getByTestId('run-list');
    expect(list.textContent).toContain('Audit deps');
    expect(list.textContent).toContain('1/2 settled');
    fireEvent.click(list.querySelector('button')!);
    expect(screen.getByText('Swarm progress')).toBeTruthy();
    expect(screen.getByTestId('progress-past').textContent).toContain('read-only');
    expect(screen.queryByTestId('graph-stop')).toBeNull();
    expect(screen.queryByTestId('progress-log')).toBeNull();
    expect(screen.getAllByTestId('selected-chip')).toHaveLength(1);
    await waitFor(() => screen.getByTestId('progress-chart'));
    fireEvent.click(screen.getByText('a'));
    fireEvent.click(screen.getByTestId('step-result'));
    expect(f.graphOutput).toHaveBeenCalledWith('a', 'g-0');
    await waitFor(() => expect(screen.getByTestId('step-result-panel').textContent).toContain('lodash fine'));
    // Back to the live run.
    fireEvent.click(screen.getByTestId('run-current'));
    expect(screen.getByText('Graph progress')).toBeTruthy();
  });

  it('no picker when the bridge has no history (older bridge / none yet)', async () => {
    const f = fakeChat({ lhProgress: live(detailed) });
    f.graphHistory.mockResolvedValue({ error: 'Earlier runs needs a newer unipi on the PC.' });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    await Promise.resolve();
    expect(screen.queryByTestId('progress-runs')).toBeNull();
  });

  it('a finished run with no steps says so (no "no details kept" copy)', () => {
    const f = fakeChat({ lhProgress: { v: 1, mode: 'none', last: { ...graph, status: 'stopped', items: [], counts: { total: 0, done: 0, running: 0, failed: 0, queued: 0 } }, log: [], updatedAt: Date.now() } });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByText('This run ended before declaring any steps.')).toBeTruthy();
    expect(screen.queryByText(/No item details kept/)).toBeNull();
  });

  it('idle copy names a sticky mode that has no run yet', () => {
    const f = fakeChat({ lhMode: { mode: 'graph' }, lhProgress: { v: 1, mode: 'none', last: { ...detailed, status: 'done', endedAt: Date.now() - 60_000 }, log: [], updatedAt: Date.now() } });
    render(() => <ProgressSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('progress-idle').textContent).toContain('Graph mode is on, no run yet · last run ended 1m ago');
  });
});

describe('ChatHeaderMenu Progress row (UNI-222)', () => {
  const anchor = { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
  it('shows "Progress" with a live hint and opens it', () => {
    const onProgress = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={onDismiss} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename onOpenSection={() => {}} onProgress={onProgress} progressLive />
    ));
    expect(screen.getByText('live')).toBeTruthy();
    fireEvent.click(screen.getByText('Progress'));
    expect(onProgress).toHaveBeenCalled();
    expect(onDismiss).toHaveBeenCalled();
  });
  it('no Progress row without onProgress', () => {
    render(() => <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename onOpenSection={() => {}} />);
    expect(screen.queryByText('Progress')).toBeNull();
  });
});
