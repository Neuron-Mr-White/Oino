/** The composer glow tier: only X-high and Max glow (Max the stronger), never read-only. */
import { describe, expect, it } from 'vitest';
import { thinkGlow } from './thinkGlow';

describe('thinkGlow', () => {
  it('names the tier for X-high and Max only', () => {
    expect(thinkGlow('max')).toBe('max');
    expect(thinkGlow('xhigh')).toBe('xhigh');
    expect(thinkGlow('X-high')).toBe('xhigh');
    for (const level of ['off', 'minimal', 'low', 'medium', 'high', '', undefined]) expect(thinkGlow(level)).toBeUndefined();
  });
  it('stays off on a read-only composer', () => {
    expect(thinkGlow('max', true)).toBeUndefined();
  });
});
