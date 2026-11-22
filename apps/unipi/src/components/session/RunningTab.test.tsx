/**
 * [running-tab] the Subagents list windows a big graph run: past RUNS_VIRTUAL_AT
 * rows only the rows in view mount (bounded DOM), newest first.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { scopedSubagents } from '../../lib/status/scopedSubagents';
import { parseSubagentRuns } from './modalData';
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import type { SessionHandle } from '../../lib/session/store';
import type { RunningData } from '../../lib/status/running';
import { RunningTab, RUNS_VIRTUAL_AT } from './RunningTab';
import type { SubagentRun } from './modalData';

afterEach(cleanup);

const handle = { hostId: 'h', sessionId: () => 's', key: 'h\u0000s', state: { commands: [], info: {}, topics: {}, conn: 'live', seq: 0 }, cmd: async () => ({}) } as unknown as SessionHandle;
const runs = (n: number): SubagentRun[] =>
  Array.from({ length: n }, (_, i) => ({ id: `r${i}`, title: `Run ${i}`, profile: 'subagent_explore', model: 'zai/glm-5.3', status: i % 3 === 0 ? 'running' : 'completed', background: true, startedAt: 1_000 + i, toolCalls: i, depth: 1 }));
const view = (n: number) => {
  const data: RunningData = { bash: [], agents: [], runs: runs(n), tabs: [] };
  render(() => <RunningTab s={handle} running={{ data: () => data, refetch: () => {} }} />);
  // The subagent filter opens on "Running" (owner 2026-11-06); the windowing
  // tests below want every row, so switch the filter to "All".
  fireEvent.click(screen.getByTestId('sm-filter-all'));
};

describe('RunningTab subagents list', () => {
  it('renders only the current session and updates one group when all finish', () => {
    const records = Array.from({ length: 5 }, (_, i) => ({ id: `a${i}`, title: `Agent ${i}`, parentSessionId: 'A', status: i < 3 ? 'running' : 'completed' }));
    const [topic, setTopic] = createSignal({ parentSessionId: 'A', records });
    const data = (id: string): RunningData => ({ bash: [], agents: [], tabs: [], runs: parseSubagentRuns(scopedSubagents(topic(), id)) });
    const a = render(() => <RunningTab s={handle} running={{ data: () => data('A'), refetch: () => {} }} />);
    expect(a.getAllByTestId('sm-sec-agents')).toHaveLength(1);
    expect(a.getByTestId('sm-agents-summary').textContent).toBe('3 running · 5 total');
    const b = render(() => <RunningTab s={handle} running={{ data: () => data('B'), refetch: () => {} }} />);
    expect(b.queryByTestId('sm-sec-agents')).toBeNull();
    setTopic({ parentSessionId: 'A', records: records.map((r) => ({ ...r, status: 'completed' })) });
    expect(a.getByTestId('sm-agents-summary').textContent).toBe('All 5 done');
    expect(b.queryByTestId('sm-sec-agents')).toBeNull();
  });
  it('the status filter opens on "Running" (in progress)', () => {
    const data: RunningData = { bash: [], agents: [], runs: runs(4), tabs: [] };
    render(() => <RunningTab s={handle} running={{ data: () => data, refetch: () => {} }} />);
    expect(screen.getByTestId('sm-filter-running').getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByTestId('sm-filter-all').getAttribute('aria-pressed')).toBe('false');
  });
  it('up to the threshold: every row, newest first, no scroll box', () => {
    view(RUNS_VIRTUAL_AT);
    const rows = screen.getAllByTestId('sm-run-row');
    expect(rows).toHaveLength(RUNS_VIRTUAL_AT);
    expect(rows[0]!.getAttribute('data-run')).toBe(`run:r${RUNS_VIRTUAL_AT - 1}`);
    expect(screen.queryByTestId('sm-runs-scroll')).toBeNull();
  });

  it('past the threshold: a windowed box mounts only the rows in view', () => {
    view(400);
    expect(screen.getByTestId('sm-runs-scroll')).toBeTruthy();
    const mounted = screen.getAllByTestId('sm-run-row').length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(40);
    expect(screen.getByTestId('sm-filter-all').textContent).toBe('All400');
  });

  it('before the first fetch: a skeleton, never a blank tab', () => {
    render(() => <RunningTab s={handle} running={{ data: () => undefined, refetch: () => {} }} />);
    expect(screen.getByTestId('sm-running-loading')).toBeTruthy();
    expect(screen.queryByTestId('sm-nothing')).toBeNull();
  });

  it('nothing at all: one quiet line', () => {
    const data: RunningData = { bash: [], agents: [], tabs: [] };
    render(() => <RunningTab s={handle} running={{ data: () => data, refetch: () => {} }} />);
    expect(screen.getByTestId('sm-nothing').textContent).toContain('Nothing running');
  });
});
