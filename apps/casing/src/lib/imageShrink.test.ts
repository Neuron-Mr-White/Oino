import { describe, it, expect } from 'vitest';
import { targetSize, MAX_SIDE } from './imageShrink';

describe('targetSize', () => {
  it('leaves images already within the cap unchanged', () => {
    expect(targetSize(800, 600)).toEqual({ width: 800, height: 600 });
    expect(targetSize(MAX_SIDE, 100)).toEqual({ width: MAX_SIDE, height: 100 });
  });

  it('shrinks the longest side to the cap, keeping aspect ratio', () => {
    expect(targetSize(3136, 2352)).toEqual({ width: 1568, height: 1176 });
    expect(targetSize(2000, 4000)).toEqual({ width: 784, height: 1568 });
  });

  it('a square over the cap shrinks evenly', () => {
    expect(targetSize(3000, 3000)).toEqual({ width: 1568, height: 1568 });
  });

  it('never rounds a dimension below 1', () => {
    const { width, height } = targetSize(100000, 1);
    expect(width).toBe(1568);
    expect(height).toBeGreaterThanOrEqual(1);
  });

  it('handles a custom cap', () => {
    expect(targetSize(2000, 1000, 500)).toEqual({ width: 500, height: 250 });
  });

  it('degenerate (zero/negative) sizes never throw and stay at least 1px', () => {
    expect(targetSize(0, 100)).toEqual({ width: 1, height: 100 });
    expect(targetSize(100, 0)).toEqual({ width: 100, height: 1 });
  });
});
