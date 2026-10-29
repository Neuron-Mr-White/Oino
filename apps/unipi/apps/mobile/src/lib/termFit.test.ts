import { describe, expect, it } from 'vitest';
import { computeFit } from './termFit';

describe('computeFit', () => {
  it('fits columns/rows from the content box without a scrollbar gutter', () => {
    const result = computeFit({
      containerWidth: 360,
      containerHeight: 624,
      cellWidth: 8.4,
      cellHeight: 21.5,
      basePaddingX: 6,
      basePaddingY: 0,
    });
    expect(result).toBeDefined();
    // (360 - 12) / 8.4 = 41.4 -> 41 cols; old FitAddon math (with the
    // 14px scrollbar gutter) would give (360-12-14)/8.4 = 39 cols.
    expect(result!.cols).toBe(41);
    expect(result!.rows).toBe(29);
  });

  it('splits the leftover pixel remainder evenly as padding on both sides', () => {
    const result = computeFit({
      containerWidth: 360,
      containerHeight: 624,
      cellWidth: 8.4,
      cellHeight: 21.5,
      basePaddingX: 6,
      basePaddingY: 0,
    });
    // 41 cols * 8.4 = 344.4px used; leftover = 360 - 344.4 = 15.6px,
    // split evenly -> 7.8px each side (vs the asymmetric 6px/~20px
    // the old scrollbar-reserving math produced).
    expect(result!.paddingX).toBeCloseTo(7.8, 1);
  });

  it('returns undefined for a zero cell size (not yet measured)', () => {
    expect(
      computeFit({
        containerWidth: 360,
        containerHeight: 624,
        cellWidth: 0,
        cellHeight: 0,
        basePaddingX: 6,
        basePaddingY: 0,
      }),
    ).toBeUndefined();
  });

  it('never proposes fewer than 2 cols or 1 row', () => {
    const result = computeFit({
      containerWidth: 10,
      containerHeight: 5,
      cellWidth: 8.4,
      cellHeight: 21.5,
      basePaddingX: 6,
      basePaddingY: 0,
    });
    expect(result!.cols).toBeGreaterThanOrEqual(2);
    expect(result!.rows).toBeGreaterThanOrEqual(1);
  });
});
