import { describe, expect, it } from 'vitest';
import { barShares, cacheHitRate, compactTokens, costNote, costText, dayLabel, peakIndex, projectLabel } from './usage';

describe('usage formatting', () => {
  it('never shows an unpriced total as $0.00', () => {
    expect(costText(0, 5000, 5000)).toBe('—');
    expect(costText(0, 0, 0)).toBe('$0.00');
    expect(costText(12.4, 0, 100)).toBe('$12.40');
    expect(costText(0.0042, 0, 100)).toBe('$0.0042');
    expect(costNote(0, 1_200_000)).toBe('1.2M tokens, no price');
    expect(costNote(3, 1500)).toBe('+ 1.5k tokens unpriced');
    expect(costNote(3, 0)).toBeUndefined();
  });
  it('compacts tokens and labels days and projects', () => {
    expect(compactTokens(845)).toBe('845');
    expect(compactTokens(34_500)).toBe('35k');
    expect(compactTokens(1_234_567)).toBe('1.2M');
    expect(dayLabel('2026-09-12')).toBe('Sep 12');
    expect(projectLabel('/home/u/Projects/app/')).toEqual({ name: 'app', parent: '/home/u/Projects' });
    expect(projectLabel('')).toEqual({ name: 'unknown', parent: '' });
  });
  it('scales bars against the peak', () => {
    expect(barShares([0, 5, 10])).toEqual([0, 0.5, 1]);
    expect(barShares([0, 0])).toEqual([0, 0]);
    expect(peakIndex([1, 9, 9, 2])).toBe(1);
    expect(peakIndex([0, 0])).toBe(-1);
    expect(cacheHitRate({ input: 100, cache_read: 300, cache_write: 100 })).toBe(0.6);
    expect(cacheHitRate({ input: 0, cache_read: 0, cache_write: 0 })).toBeUndefined();
  });
});
