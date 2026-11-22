// @vitest-environment jsdom
/**
 * Settings › Memory › Dream (v3 J3): settings form visible without a toggle, a status line
 * with the next scheduled run, Recent dreams (duration, memory changes, proposals) and
 * approve / reject / report through the `dream.*` services.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';

const calls: { name: string; payload?: unknown }[] = [];
let status: unknown;

vi.mock('../../lib/settings/run', () => ({
  settingsCommandChat: async () => ({
    cmd: async (c: { name: string; payload?: unknown }) => {
      calls.push({ name: c.name, payload: c.payload });
      if (c.name === 'dream.status') return status;
      if (c.name === 'dream.detail') return { report: '## Memory edits\n- merged a + b' };
      return { ok: true, message: 'done' };
    },
  }),
}));
vi.mock('../../ui/settings/data', () => ({ settingsSchema: async () => ({ ok: true, value: [{ namespace: 'dream', sections: [] }] }) }));
vi.mock('../../ui/settings/form', () => ({ NamespaceForm: (p: { testId?: string }) => <div data-testid={p.testId}>form</div> }));

import { changeParts, DreamPanel, duration, parseDreamStatus, statusLine, whenNext } from './DreamPanel';

const T0 = Date.UTC(2026, 10, 6, 3, 0, 0);
const base = {
  v: 2,
  enabled: true,
  schedule: { mode: 'daily', time: '03:00', intervalHours: 24 },
  lastRunAt: T0,
  running: false,
  scheduler: { registered: true, hostRunning: true, nextRun: new Date(Date.now() + 3 * 3600_000).toISOString() },
  runs: [
    {
      id: 'r1', status: 'done', startedAt: T0, endedAt: T0 + 192_000, manual: false, sessions: 4, events: 9, hasReport: true,
      memoryChanges: { added: 2, updated: 0, merged: 3, pruned: 1 },
      proposals: [{ id: 'p1', name: 'fix-lint', kind: 'skill', decision: 'pending' }],
      summary: ['merged 3 notes'],
    },
    { id: 'r2', status: 'failed', startedAt: T0 - 86400_000, manual: true, sessions: 0, events: 0, proposals: [], error: 'timed out' },
  ],
};

beforeEach(() => {
  calls.length = 0;
  status = base;
});
afterEach(cleanup);

describe('pure helpers', () => {
  it('parses nextRun (ISO or seconds) and memory changes', () => {
    const s = parseDreamStatus({ ...base, scheduler: { registered: true, hostRunning: true, nextRun: 1_800_000_000 } });
    expect(s.scheduler?.nextRun).toBe(1_800_000_000_000);
    expect(s.runs[0]!.memoryChanges).toEqual({ added: 2, updated: 0, merged: 3, pruned: 1 });
    expect(s.scheduleMode).toBe('daily');
  });
  it('formats duration and non-zero change chips', () => {
    expect(duration({ startedAt: T0, endedAt: T0 + 192_000 })).toBe('3m 12s');
    expect(duration({ startedAt: T0 })).toBeUndefined();
    expect(changeParts({ added: 2, updated: 0, merged: 3, pruned: 0 }).map((c) => c.label)).toEqual(['2 added', '3 merged']);
  });
  it('status line covers off, manual, no host, next run', () => {
    const now = Date.UTC(2026, 10, 6, 12);
    const s = parseDreamStatus(base);
    expect(statusLine({ ...s, enabled: false }, now).text).toMatch(/off/);
    expect(statusLine({ ...s, scheduleMode: 'manual' }, now).text).toMatch(/Manual only/);
    expect(statusLine({ ...s, scheduler: { registered: true, hostRunning: false } }, now).tone).toBe('warn');
    const next = Date.UTC(2026, 10, 7, 3);
    expect(statusLine({ ...s, scheduler: { registered: true, hostRunning: true, nextRun: next } }, now).text).toMatch(/^Next dream: tomorrow/);
    expect(whenNext(now, now)).toMatch(/^today/);
  });
});

describe('DreamPanel', () => {
  it('shows settings form and next run without any toggle', async () => {
    render(() => <DreamPanel hostId="h" />);
    await waitFor(() => screen.getByTestId('dream-settings-form'));
    expect(screen.queryByTestId('dream-settings-toggle')).toBeNull();
    expect(screen.getByTestId('dream-status-line').textContent).toMatch(/Next dream/);
  });

  it('lists recent dreams with duration, changes and proposals; approves a proposal', async () => {
    render(() => <DreamPanel hostId="h" />);
    await waitFor(() => screen.getByTestId('dream-run-r1'));
    const row = screen.getByTestId('dream-run-r1');
    expect(row.textContent).toContain('3m 12s');
    expect(screen.getByTestId('dream-changes-r1').textContent).toContain('2 added');
    expect(screen.getByTestId('dream-changes-r1').textContent).toContain('1 pruned');
    expect(row.textContent).toContain('1 proposal');
    fireEvent.click(screen.getByTestId('dream-approve-p1')); // pending run starts expanded
    await waitFor(() => expect(calls.some((c) => c.name === 'dream.approve')).toBe(true));
    expect(calls.find((c) => c.name === 'dream.approve')!.payload).toEqual({ runId: 'r1', proposalId: 'p1' });
  });

  it('rejects, and opens the per-run report', async () => {
    render(() => <DreamPanel hostId="h" />);
    await waitFor(() => screen.getByTestId('dream-reject-p1'));
    fireEvent.click(screen.getByTestId('dream-reject-p1'));
    await waitFor(() => expect(calls.some((c) => c.name === 'dream.reject')).toBe(true));
    fireEvent.click(screen.getByTestId('dream-report-r1'));
    await waitFor(() => expect(calls.some((c) => c.name === 'dream.detail')).toBe(true));
  });

  it('expands a failed run to show its error', async () => {
    render(() => <DreamPanel hostId="h" />);
    await waitFor(() => screen.getByTestId('dream-run-toggle-r2'));
    fireEvent.click(screen.getByTestId('dream-run-toggle-r2'));
    expect(screen.getByTestId('dream-run-r2').textContent).toContain('timed out');
  });
});
