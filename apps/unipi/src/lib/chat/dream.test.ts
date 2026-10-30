import { describe, expect, it } from 'vitest';
import { applyMessage } from './store';
import { durationText, isDreamStatus, pendingCount, runFacts, startedText, statusLine, type DreamStatus } from './dream';

const st = (x: Partial<DreamStatus> = {}): DreamStatus => ({ v: 1, enabled: true, minSessions: 5, minGapHours: 12, lastRunAt: 0, running: false, runs: [], ...x });

describe('dream helpers', () => {
  it('validates the wire shape', () => {
    expect(isDreamStatus(st())).toBe(true);
    expect(isDreamStatus({ v: 2, runs: [], enabled: true })).toBe(false);
    expect(isDreamStatus(undefined)).toBe(false);
  });
  it('formats durations, starts and facts', () => {
    expect(durationText(42_000)).toBe('42s');
    expect(durationText(185_000)).toBe('3m 05s');
    expect(durationText(4_320_000)).toBe('1h 12m');
    const now = new Date(2026, 9, 10, 15, 0).getTime();
    expect(startedText(new Date(2026, 9, 10, 14, 2).getTime(), now)).toBe('Today 14:02');
    expect(startedText(new Date(2026, 9, 9, 9, 10).getTime(), now)).toBe('Yesterday 09:10');
    expect(runFacts({ id: 'a', status: 'finished', startedAt: 0, endedAt: 65_000, manual: false, sessions: 1, events: 2, pending: 0, hasReport: true, proposals: [{ id: 'p', name: 'p', kind: 'skill', decision: 'approved' }] })).toBe('1 session · 1 proposal · 1m 05s');
  });
  it('status line covers off / running / due / last run', () => {
    expect(statusLine(st({ enabled: false }))).toBe('Background dreaming is off');
    expect(statusLine(st({ running: true }))).toMatch(/running/);
    expect(statusLine(st({ due: { due: true, reason: 'x' } }))).toMatch(/Due/);
    expect(statusLine(st({ due: { due: false, reason: 'needs 2 more new sessions' } }))).toBe('No dream yet · needs 2 more new sessions');
    expect(pendingCount(st({ runs: [{ pending: 2 } as never, { pending: 1 } as never] }))).toBe(3);
  });
  it('store: hello.dream and dream_status pushes', () => {
    const s = { entries: [], pending: [], run: { running: false } } as never as Parameters<typeof applyMessage>[0];
    applyMessage(s, { t: 'hello', session: {}, state: { running: false }, entries: [], dream: st({ enabled: false }) });
    expect(s.dream?.enabled).toBe(false);
    expect(s.dreamSupported).toBe(true);
    applyMessage(s, { t: 'dream_status', status: st({ running: true }) });
    expect(s.dream?.running).toBe(true);
    applyMessage(s, { t: 'dream_status', status: undefined });
    expect(s.dream).toBeUndefined();
    applyMessage(s, { t: 'hello', session: {}, state: { running: false }, entries: [] });
    expect(s.dream).toBeUndefined();
  });
});
