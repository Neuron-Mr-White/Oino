import { describe, expect, it } from 'vitest';
import { arrowHead, clampRect, currentCrop, drawMarks, fitBox, hasEdits, toImage, type EditOp } from './imageEdit';

describe('image edit model (UNI-227)', () => {
  it('the last crop wins; none = whole image', () => {
    expect(currentCrop([], 800, 600)).toEqual({ x: 0, y: 0, w: 800, h: 600 });
    const ops: EditOp[] = [{ kind: 'crop', rect: { x: 10, y: 10, w: 100, h: 100 } }, { kind: 'draw', color: '#fff', width: 4, points: [] }, { kind: 'crop', rect: { x: 20, y: 30, w: 50, h: 60 } }];
    expect(currentCrop(ops, 800, 600)).toEqual({ x: 20, y: 30, w: 50, h: 60 });
  });

  it('clamps crops inside the image, flips negative drags, keeps a minimum size', () => {
    expect(clampRect({ x: 700, y: 500, w: -200, h: -100 }, 800, 600)).toEqual({ x: 500, y: 400, w: 200, h: 100 });
    expect(clampRect({ x: -50, y: -50, w: 2000, h: 2000 }, 800, 600)).toEqual({ x: 0, y: 0, w: 800, h: 600 });
    expect(clampRect({ x: 790, y: 10, w: 2, h: 2 }, 800, 600)).toEqual({ x: 784, y: 10, w: 16, h: 16 });
  });

  it('fits and maps screen points back to image pixels through the crop', () => {
    const crop = { x: 100, y: 50, w: 400, h: 200 };
    const box = fitBox(crop, 800, 800);
    expect(box.scale).toBe(2);
    expect(box.top).toBe(200);
    expect(toImage({ x: box.left, y: box.top }, box, crop)).toEqual({ x: 100, y: 50 });
    expect(toImage({ x: box.left + 800, y: box.top + 400 }, box, crop)).toEqual({ x: 500, y: 250 });
  });

  it('arrow heads sit behind the tip on both sides', () => {
    const [a, b] = arrowHead({ x: 0, y: 0 }, { x: 100, y: 0 }, 4);
    expect(a.x).toBeLessThan(100);
    expect(b.x).toBeLessThan(100);
    expect(Math.sign(a.y)).toBe(-Math.sign(b.y));
  });

  it('draws strokes and arrows; crops draw nothing', () => {
    const calls: string[] = [];
    const ctx = {
      beginPath: () => calls.push('begin'),
      moveTo: (x: number, y: number) => calls.push(`m${x},${y}`),
      lineTo: (x: number, y: number) => calls.push(`l${Math.round(x)},${Math.round(y)}`),
      stroke: () => calls.push('stroke'),
      fill: () => calls.push('fill'),
      closePath: () => calls.push('close'),
      strokeStyle: '', fillStyle: '', lineWidth: 0, lineCap: 'butt', lineJoin: 'miter',
    } as never;
    drawMarks(ctx, [{ kind: 'crop', rect: { x: 0, y: 0, w: 1, h: 1 } }, { kind: 'draw', color: '#f00', width: 2, points: [{ x: 10, y: 10 }, { x: 20, y: 20 }] }], { x: 10, y: 10 }, 2);
    expect(calls).toEqual(['begin', 'm0,0', 'l20,20', 'stroke']);
    calls.length = 0;
    drawMarks(ctx, [{ kind: 'arrow', color: '#f00', width: 2, from: { x: 0, y: 0 }, to: { x: 50, y: 0 } }]);
    expect(calls.filter((c) => c === 'stroke' || c === 'fill')).toEqual(['stroke', 'fill']);
  });

  it('hasEdits ignores a full-image crop', () => {
    expect(hasEdits([], 10, 10)).toBe(false);
    expect(hasEdits([{ kind: 'crop', rect: { x: 0, y: 0, w: 10, h: 10 } }], 10, 10)).toBe(false);
    expect(hasEdits([{ kind: 'crop', rect: { x: 1, y: 0, w: 9, h: 10 } }], 10, 10)).toBe(true);
  });
});
