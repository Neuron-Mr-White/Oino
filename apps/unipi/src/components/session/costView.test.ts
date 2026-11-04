import { describe, expect, it } from 'vitest';
import { formatUsd, parseSessionStats, sessionCostView } from './modalData';

const tokens = (total: number) => ({ input: total, output: 0, cacheRead: 0, cacheWrite: 0, total });

describe('[status] session cost (owner: "session cost is 0")', () => {
  it('a priced session (subagent runs rolled up by core) reads its dollars', () => {
    expect(sessionCostView({ cost: 71.3, tokens: tokens(4_000_000), unpricedTokens: 0 })).toEqual({ text: '$71.30', title: '$71.30, subagent runs included' });
  });

  it('an unpriced provider reads — plus its tokens, never $0.00', () => {
    const v = sessionCostView({ cost: 0, tokens: tokens(230_000), unpricedTokens: 230_000 });
    expect(v.text).toBe('—');
    expect(v.sub).toBe('230.0k tokens, no price');
    // An older host without `unpricedTokens`: cost 0 with tokens spent is unpriced too.
    expect(sessionCostView({ cost: 0, tokens: tokens(5000), unpricedTokens: 0 }).text).toBe('—');
  });

  it('a partly priced session keeps the priced sum and names the rest', () => {
    const v = sessionCostView({ cost: 2.5, tokens: tokens(2_000_000), unpricedTokens: 1_200_000 });
    expect(v.text).toBe('$2.50');
    expect(v.sub).toBe('+ 1.2M tokens unpriced');
  });

  it('nothing spent is $0.00; not loaded is —', () => {
    expect(sessionCostView({ cost: 0, tokens: tokens(0), unpricedTokens: 0 }).text).toBe('$0.00');
    expect(sessionCostView(undefined).text).toBe('—');
    expect(formatUsd(undefined)).toBe('—');
    expect(formatUsd(0.004)).toBe('$0.0040');
  });

  it('parses unpricedTokens from get_session_stats', () => {
    expect(parseSessionStats({ sessionId: 's', cost: 1, unpricedTokens: 42, tokens: { total: 50 } })!.unpricedTokens).toBe(42);
    expect(parseSessionStats({ sessionId: 's', cost: 1 })!.unpricedTokens).toBe(0);
  });
});

import { sessionModelUsage, sessionSpanMs } from './modalData';

describe('this session usage (Info › Session)', () => {
  const msg = (model: string, input: number, output: number, cost: number, ts?: string) => ({ type: 'message', timestamp: ts, message: { role: 'assistant', model, usage: { input, output, cacheRead: 5, cacheWrite: 0, cost: { total: cost } } } });
  it('groups assistant replies per model; an unpriced model keeps cost 0', () => {
    const rows = sessionModelUsage([msg('zai/glm-5', 100, 20, 0), msg('zai/glm-5', 50, 10, 0), msg('claude', 10, 5, 0.3), { type: 'message', message: { role: 'user' } }]);
    expect(rows.map((r) => [r.model, r.turns, r.tokens, r.cost])).toEqual([['glm-5', 2, 190, 0], ['claude', 1, 20, 0.3]]);
  });
  it('spans first to last stamp', () => {
    expect(sessionSpanMs([{ timestamp: '2026-09-21T10:00:00.000Z' }, { timestamp: '2026-09-21T10:05:30.000Z' }, {}])).toBe(330_000);
    expect(sessionSpanMs([{ timestamp: '2026-09-21T10:00:00.000Z' }])).toBeUndefined();
  });
});
