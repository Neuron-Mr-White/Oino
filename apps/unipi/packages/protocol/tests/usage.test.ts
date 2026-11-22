import { describe, expect, it } from 'vitest';
import { UsageDataSchema, UsageStatsRequestSchema } from '../src/index.js';

describe('usage_stats', () => {
  it('parses the host reply and bounds the request', () => {
    const data = UsageDataSchema.parse({
      days: 7,
      totals: { input: 1, output: 2, cache_read: 3, cache_write: 4, tokens: 10, cost: 0.5, unpriced_tokens: 0, turns: 2, sessions: 1, active_days: 1 },
      daily: [{ day: '2026-09-21', tokens: 10, cost: 0.5, unpriced_tokens: 0 }],
      by_model: [{ name: 'glm', tokens: 10, cost: 0.5, unpriced_tokens: 0, turns: 2 }],
      by_project: [{ name: '/p/a', tokens: 10, cost: 0.5, unpriced_tokens: 0, sessions: 1 }],
    });
    expect(data.by_model[0]?.name).toBe('glm');
    expect(UsageStatsRequestSchema.safeParse({ op: 'usage_stats', days: 400 }).success).toBe(false);
    expect(UsageStatsRequestSchema.safeParse({ op: 'usage_stats', days: 30, tz_offset_min: -120 }).success).toBe(true);
  });
});
