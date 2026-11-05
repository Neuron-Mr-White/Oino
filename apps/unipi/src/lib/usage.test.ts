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

import { heatmapColumns, heatmapMonths, lifetimeSummary, modelShares, trendSeries, durationText } from './usage';
describe('ZCode-parity usage extras', () => {
  const days = Array.from({ length: 20 }, (_, i) => ({ day: new Date(Date.UTC(2026, 9, 1 + i)).toISOString().slice(0, 10), tokens: i === 5 || i === 18 ? 0 : (i + 1) * 100, turns: i, models: { a: i * 60, b: i * 40 } }));
  it('lifetime: total, peak, streaks (empty today keeps the streak)', () => {
    const l = lifetimeSummary(days);
    expect(l.peakTokens).toBe(2000);
    expect(l.longestStreak).toBe(12);
    expect(l.currentStreak).toBe(1);
    expect(lifetimeSummary([...days.slice(0, 18)]).currentStreak).toBe(12);
  });
  it('heatmap: 52 Sunday-first columns ending on the newest day; weekly fills from the bottom', () => {
    const cols = heatmapColumns(days, 'daily');
    expect(cols).toHaveLength(52);
    expect(cols.every((c) => c.cells.length === 7)).toBe(true);
    expect(cols.at(-1)!.cells.some((c) => c.day === days.at(-1)!.day)).toBe(true);
    expect(cols.flatMap((c) => c.cells).find((c) => c.day === days.at(-1)!.day)!.level).toBe(4);
    expect(cols.flatMap((c) => c.cells).find((c) => c.day === days[5]!.day)!.level).toBe(0);
    const weekly = heatmapColumns(days, 'weekly');
    const top = weekly.reduce((a, b) => (b.total > a.total ? b : a));
    expect(top.cells.every((c) => c.level === 4)).toBe(true);
    expect(heatmapMonths(cols).reduce((s, m) => s + m.span, 0)).toBe(52);
  });
  it('trend series and donut shares', () => {
    expect(trendSeries(days)).toEqual(['a', 'b']);
    const rows = modelShares([{ name: 'x', tokens: 50 }, { name: 'y', tokens: 30 }, { name: 'z', tokens: 20 }], 2);
    expect(rows.map((r) => r.name)).toEqual(['x', 'Other models']);
    expect(durationText(3 * 86_400_000 + 4 * 3_600_000 + 60_000)).toBe('3d 4h');
  });
});
