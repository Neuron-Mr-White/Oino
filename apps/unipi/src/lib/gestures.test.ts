import { describe, expect, it } from 'vitest';
import {
  isDoubleTap,
  pinchFontSize,
  twoFingerSwipe,
} from './gestures';

describe('two-finger swipe → pane switching', () => {
  it('needs the threshold', () => {
    expect(twoFingerSwipe({ startX: 200, currentX: 210 })).toBeUndefined();
  });

  it('swipe right → previous pane, swipe left → next pane', () => {
    expect(twoFingerSwipe({ startX: 200, currentX: 300 })).toBe('pane_prev');
    expect(twoFingerSwipe({ startX: 200, currentX: 100 })).toBe('pane_next');
  });
});

describe('pinch → font size', () => {
  it('scales and clamps to 8–32', () => {
    expect(pinchFontSize(12, 1.5)).toBe(18);
    expect(pinchFontSize(12, 0.5)).toBe(8);
    expect(pinchFontSize(30, 1.5)).toBe(32);
    expect(pinchFontSize(12, 1)).toBe(12);
  });
});

describe('double-tap detection', () => {
  it('accepts two taps inside 300 ms', () => {
    expect(isDoubleTap([1000, 1150])).toBe(true);
    expect(isDoubleTap([1000, 1400])).toBe(false);
    expect(isDoubleTap([1000])).toBe(false);
  });
});
