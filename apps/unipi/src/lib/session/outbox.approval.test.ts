import { describe, expect, it } from 'vitest';
import { sweep } from './outbox';
import { initialState } from './reducer';

describe('G3-4 pending approval outbox', () => {
  it('does not expire accepted or in-flight user messages while an approval waits', () => {
    const s = initialState('h1', 's1');
    s.approvals = [{ requestId: 'approval', sessionId: 's1', tool: 'bash', arguments: {}, riskLevel: 'high', ceiling: 'medium', reason: 'needs approval' }];
    s.outbox = [
      { ref: 'sent', text: 'follow up', mode: 'followUp', status: 'sending', at: 0 },
      { ref: 'accepted', text: 'accepted', mode: 'followUp', status: 'acked', at: 0, ackedAt: 0 },
    ];
    expect(sweep(s, 120_000)).toEqual([]);
    expect(s.outbox.map((o) => o.status)).toEqual(['sending', 'acked']);
    s.approvals = [];
    expect(sweep(s, 120_001)).toEqual([]);
    expect(s.outbox).toHaveLength(2);
    expect(sweep(s, 160_001)).toEqual(['sent']);
    expect(s.outbox).toHaveLength(1);
  });
});
