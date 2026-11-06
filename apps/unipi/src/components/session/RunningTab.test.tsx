/**
 * [running-tab] the Subagents list windows a big graph run: past RUNS_VIRTUAL_AT
 * rows only the rows in view mount (bounded DOM), newest first.
 */
import { afterEach, describe, expect, it } from 'vitest';
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
