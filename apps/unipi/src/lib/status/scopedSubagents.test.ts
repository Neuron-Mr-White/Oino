import { describe, expect, it } from 'vitest';
import { scopedSubagents } from './scopedSubagents';
import { topicRunningCount, topicRunningTrayLabel } from './running';
import { parseSubagentRuns } from '../../components/session/modalData';
import { runRowsOf } from './running';

describe('session scoped subagents', () => {
  it('keeps one coherent group and ignores records from another session', () => {
    const records = Array.from({ length: 5 }, (_, i) => ({ id: `a${i}`, parentSessionId: 'A', status: i < 3 ? 'running' : 'completed' }));
    const value = { parentSessionId: 'A', records: [...records, { id: 'b', parentSessionId: 'B', status: 'running' }] };
    const a = parseSubagentRuns(scopedSubagents(value, 'A'))!;
    const b = parseSubagentRuns(scopedSubagents(value, 'B'))!;
    expect(a).toHaveLength(5);
    expect(b).toHaveLength(0);
    const data = { bash: [], agents: [], runs: a, tabs: [] };
    expect(runRowsOf(data)).toHaveLength(5);
    const topics = { 'unipi:subagents:list': { data: value, sticky: true, publisher: '' } };
    expect(topicRunningCount(topics, 'A')).toBe(3);
    expect(topicRunningCount(topics, 'B')).toBe(0);
    expect(topicRunningTrayLabel(topics, 'A')).toBe('Subagents 3 running');
    expect(topicRunningTrayLabel(topics, 'B')).toBeUndefined();
    expect(scopedSubagents({ records: [{ id: 'legacy' }] }, 'A')).toHaveLength(1);
  });
});
