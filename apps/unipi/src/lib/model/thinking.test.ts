import { describe, expect, it } from 'vitest';
import { clampLevel, levelsOfModel } from './thinking';
import { hits } from './picker';

describe('levelsOfModel / clampLevel (D-02)', () => {
  it('a model without reasoning offers only off; xhigh / max need the map', () => {
    expect(levelsOfModel({ reasoning: false })).toEqual(['off']);
    expect(levelsOfModel({ reasoning: true })).toEqual(['off', 'minimal', 'low', 'medium', 'high']);
    expect(levelsOfModel({ reasoning: true, thinkingLevelMap: { minimal: null, medium: null, xhigh: 'max' } })).toEqual(['off', 'low', 'high', 'xhigh']);
    expect(levelsOfModel({})).toBeUndefined();
  });
  it('clamps like the host: nearest at or above, else the highest below', () => {
    expect(clampLevel('medium', ['off', 'low', 'high', 'max'])).toBe('high');
    expect(clampLevel('max', ['off', 'low', 'high'])).toBe('high');
    expect(clampLevel('low', ['off', 'low'])).toBe('low');
  });
});

describe('model search (D-25)', () => {
  it('matches every word in any order', () => {
    const m = { provider: 'oino', id: 'deepseek-v4.1-flash', name: 'DeepSeek V4.1 Flash' };
    expect(hits(m, 'deepseek flash')).toBe(true);
    expect(hits(m, 'flash deepseek')).toBe(true);
    expect(hits(m, 'deep seek')).toBe(true);
    expect(hits(m, 'deepseek pro')).toBe(false);
  });
});
