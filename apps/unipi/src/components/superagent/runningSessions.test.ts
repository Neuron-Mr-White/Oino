import { describe, expect, it } from 'vitest';
import { StatusSchema, SuperagentSessionSchema } from './api';
import { runningSessions } from './runningSessions';

const lead = (sessionId: string, projectId: string) => ({ sessionId, projectId, autonomy: 'high', ticketIds: [] });
const status = (leads: object[]) => StatusSchema.parse({ enabled: true, concurrencyCap: 3, leads, projects: [{ projectId: 'a', name: 'Alpha', autoStart: true }, { projectId: 'b', name: 'Beta', autoStart: true }] });
const history = (sessionId: string, project: string, kind?: string) => SuperagentSessionSchema.parse({ sessionId, project, title: sessionId, createdAt: 1, updatedAt: 2, path: '/mock', ...(kind ? { state: { kind } } : {}) });

describe('runningSessions', () => {
  it('merges leads and manual runs, deduplicates, and follows start, switch and stop', () => {
    const leads = [lead('lead-a', 'a'), lead('lead-b', 'b')];
    const records = [history('lead-a', 'Alpha', 'working'), history('manual', 'Gamma', 'working'), history('old', 'Old')];
    let rows = runningSessions(status(leads), records, 'manual');
    expect(rows).toHaveLength(3);
    expect(Object.fromEntries(rows.map((r) => [r.sessionId, r.project]))).toEqual({ 'lead-a': 'Alpha', 'lead-b': 'Beta', manual: 'Gamma' });
    expect(rows.filter((r) => r.open).map((r) => r.sessionId)).toEqual(['manual']);
    rows = runningSessions(status(leads), [...records, history('new', 'Delta', 'working')], 'new');
    expect(rows).toHaveLength(4);
    expect(rows.find((r) => r.open)?.sessionId).toBe('new');
    rows = runningSessions(status([leads[1]!]), [...records.filter((r) => r.sessionId !== 'lead-a'), history('new', 'Delta', 'working')], 'new');
    expect(rows.map((r) => r.sessionId).sort()).toEqual(['lead-b', 'manual', 'new']);
  });
  it('accepts old records without state and wake-capable pending work', () => {
    const old = history('old', 'Old');
    expect(runningSessions(undefined, [old])).toEqual([]);
    expect(runningSessions(undefined, [{ ...old, pendingWork: 'graph' }])).toHaveLength(1);
  });
});
