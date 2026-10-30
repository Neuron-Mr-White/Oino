import { describe, expect, it } from 'vitest';
import {
  arrowSequence,
  cellAt,
  clickSequence,
  decayVelocity,
  linesPerWheel,
  MOMENTUM_FRICTION,
  MOMENTUM_MIN_VELOCITY,
  MOMENTUM_START_VELOCITY,
  SCROLL_GAIN,
  scrollStepPx,
  releaseVelocity,
  ScrollAccumulator,
  scrollTargetFor,
  wheelSequence,
} from './termScroll';

describe('scrollTargetFor', () => {
  it('sends wheel reports whenever the app tracks the mouse', () => {
    expect(scrollTargetFor('vt200', 'alternate')).toBe('mouse');
    expect(scrollTargetFor('any', 'normal')).toBe('mouse');
    expect(scrollTargetFor('drag', 'alternate')).toBe('mouse');
  });
  it('uses arrow keys on the alt screen without mouse tracking', () => {
    expect(scrollTargetFor('none', 'alternate')).toBe('alt-keys');
  });
  it('scrolls local scrollback on the normal screen', () => {
    expect(scrollTargetFor('none', 'normal')).toBe('scrollback');
  });
});

describe('wheelSequence', () => {
  it('encodes SGR wheel up/down at the cell', () => {
    expect(wheelSequence('up', 12, 7, 'SGR')).toBe('\x1b[<64;12;7M');
    expect(wheelSequence('down', 1, 1, 'SGR')).toBe('\x1b[<65;1;1M');
  });
  it('encodes SGR pixel positions', () => {
    expect(wheelSequence('up', 3, 4, 'SGR_PIXELS', { x: 40.4, y: 99.6 })).toBe('\x1b[<64;40;100M');
  });
  it('encodes legacy X10 bytes offset by 32', () => {
    expect(wheelSequence('up', 1, 1, 'DEFAULT')).toBe('\x1b[M`!!');
    expect(wheelSequence('down', 10, 5, 'DEFAULT')).toBe('\x1b[Ma*%');
    // positions beyond 223 are clamped so the byte stays valid
    expect(wheelSequence('up', 500, 2, 'DEFAULT').charCodeAt(4)).toBe(255);
  });
});

describe('arrowSequence', () => {
  it('honours application cursor mode', () => {
    expect(arrowSequence('up', false)).toBe('\x1b[A');
    expect(arrowSequence('down', false)).toBe('\x1b[B');
    expect(arrowSequence('up', true)).toBe('\x1bOA');
  });
});

describe('cellAt', () => {
  const cell = { width: 8, height: 16 };
  const size = { cols: 40, rows: 30 };
  it('maps points to 1-based cells', () => {
    expect(cellAt({ x: 10 + 0, y: 20 + 0 }, { left: 10, top: 20 }, cell, size)).toEqual({ col: 1, row: 1 });
    expect(cellAt({ x: 10 + 17, y: 20 + 33 }, { left: 10, top: 20 }, cell, size)).toEqual({ col: 3, row: 3 });
  });
  it('clamps outside points', () => {
    expect(cellAt({ x: -50, y: 9999 }, { left: 0, top: 0 }, cell, size)).toEqual({ col: 1, row: 30 });
  });
});

describe('ScrollAccumulator', () => {
  it('emits one step per step distance and keeps the remainder', () => {
    const acc = new ScrollAccumulator(30);
    expect(acc.push(20)).toBe(0);
    expect(acc.push(20)).toBe(1); // 40 → one step, 10 left
    expect(acc.push(-50)).toBe(-1); // -40 → one step down, -10 left
    expect(acc.push(95)).toBe(2); // 85 → two steps
  });
  it('reset drops the remainder', () => {
    const acc = new ScrollAccumulator(30);
    acc.push(29);
    acc.reset();
    expect(acc.push(2)).toBe(0);
  });
});

describe('momentum helpers', () => {
  it('velocity uses the recent window only', () => {
    const samples = [
      { t: 0, y: 0 },
      { t: 500, y: 10 },
      { t: 550, y: 60 },
      { t: 580, y: 90 },
    ];
    expect(releaseVelocity(samples)).toBeCloseTo(80 / 80);
    expect(releaseVelocity([{ t: 0, y: 0 }])).toBe(0);
  });
  it('decays with friction', () => {
    expect(decayVelocity(1, 16)).toBeCloseTo(MOMENTUM_FRICTION);
    expect(decayVelocity(1, 32)).toBeCloseTo(MOMENTUM_FRICTION * MOMENTUM_FRICTION);
  });
  it('lines per wheel follows the mux', () => {
    expect(linesPerWheel('tmux')).toBe(5);
    expect(linesPerWheel('herdr')).toBe(3);
    expect(linesPerWheel(undefined)).toBe(3);
  });
});

describe('scroll sensitivity (UNI-127)', () => {
  it('moves more than one line per cell height of finger travel', () => {
    expect(SCROLL_GAIN).toBeGreaterThan(1);
    // local scrollback / arrows: one line per cellHeight / gain px
    expect(scrollStepPx('scrollback', 16, undefined)).toBeCloseTo(10);
    expect(scrollStepPx('alt-keys', 16, 'tmux')).toBeCloseTo(10);
  });
  it('wheel steps cover a whole notch of lines', () => {
    // herdr: 3 lines per notch → 48 / 1.6 = 30 px per notch
    expect(scrollStepPx('mouse', 16, 'herdr')).toBeCloseTo(30);
    // tmux: 5 lines per notch → 80 / 1.6 = 50 px
    expect(scrollStepPx('mouse', 16, 'tmux')).toBeCloseTo(50);
    // never below 1 px
    expect(scrollStepPx('mouse', 0, 'tmux')).toBe(1);
  });
  it('momentum glides ~1.5 s and starts on gentle flicks', () => {
    let v = 1;
    let t = 0;
    while (v > MOMENTUM_MIN_VELOCITY) {
      v = decayVelocity(v, 16);
      t += 16;
    }
    expect(t).toBeGreaterThan(1200);
    expect(t).toBeLessThan(2000);
    expect(MOMENTUM_START_VELOCITY).toBeLessThan(0.35);
  });
});

describe('clickSequence (tap = mouse click)', () => {
  it('SGR: press M + release m at the cell', () => {
    expect(clickSequence(12, 3, 'SGR')).toBe('\x1b[<0;12;3M\x1b[<0;12;3m');
  });
  it('SGR pixels: uses the pixel offset', () => {
    expect(clickSequence(12, 3, 'SGR_PIXELS', { x: 100.4, y: 40.6 })).toBe('\x1b[<0;100;41M\x1b[<0;100;41m');
  });
  it('X10: button 0 press, button 3 release, 32-offset coordinates', () => {
    expect(clickSequence(1, 1, 'DEFAULT')).toBe('\x1b[M !!\x1b[M#!!');
  });
  it('X10-only tracking reports the press alone', () => {
    expect(clickSequence(2, 2, 'DEFAULT', undefined, false)).toBe('\x1b[M ""');
  });
});
