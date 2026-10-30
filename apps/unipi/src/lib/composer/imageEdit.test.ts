/**
 * Image editor model (WP-A13, D-5): the crop/arrow/stroke rules from
 * UNI-227 plus the ellipse tool — bbox drag, Shift = circle, hit test, and
 * the flattened-PNG draw serialisation (`drawMarks`, the same call the
 * editor's `done()` makes on its output canvas).
 */
import { describe, expect, it } from 'vitest';
import {
  arrowHead,
  clampRect,
  currentCrop,
  drawMarks,
  ellipseAt,
  ellipseGeometry,
  ellipseRect,
  fitBox,
  hasEdits,
  pointInEllipse,
  pointOnEllipse,
  toImage,
  type EditOp,
  type Ellipse,
} from './imageEdit';

/** Records what `drawMarks` asks the canvas to paint (no jsdom canvas). */
function recorder() {
  const calls: string[] = [];
  const ctx = {
    beginPath: () => calls.push('begin'),
    moveTo: (x: number, y: number) => calls.push(`m${x},${y}`),
    lineTo: (x: number, y: number) => calls.push(`l${Math.round(x)},${Math.round(y)}`),
    ellipse: (x: number, y: number, rx: number, ry: number, rot: number, from: number, to: number) =>
      calls.push(`e${Math.round(x)},${Math.round(y)},${Math.round(rx)},${Math.round(ry)},${rot},${from},${to}`),
    stroke: () => calls.push('stroke'),
    fill: () => calls.push('fill'),
    closePath: () => calls.push('close'),
    strokeStyle: '',
    fillStyle: '',
    lineWidth: 0,
    lineCap: 'butt',
    lineJoin: 'miter',
  } as never;
  return { ctx, calls };
}

describe('image edit model (UNI-227)', () => {
  it('the last crop wins; none = whole image', () => {
    expect(currentCrop([], 800, 600)).toEqual({ x: 0, y: 0, w: 800, h: 600 });
    const ops: EditOp[] = [
      { kind: 'crop', rect: { x: 10, y: 10, w: 100, h: 100 } },
      { kind: 'draw', color: '#fff', width: 4, points: [] },
      { kind: 'crop', rect: { x: 20, y: 30, w: 50, h: 60 } },
    ];
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
    const { ctx, calls } = recorder();
    drawMarks(
      ctx,
      [
        { kind: 'crop', rect: { x: 0, y: 0, w: 1, h: 1 } },
        { kind: 'draw', color: '#f00', width: 2, points: [{ x: 10, y: 10 }, { x: 20, y: 20 }] },
      ],
      { x: 10, y: 10 },
      2,
    );
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

describe('ellipse tool (D-5)', () => {
  const op = (from: { x: number; y: number }, to: { x: number; y: number }): Ellipse => ({ kind: 'ellipse', color: '#F07818', width: 4, from, to });

  it('a drag spans a bounding box, from either direction', () => {
    expect(ellipseRect({ x: 10, y: 20 }, { x: 110, y: 70 })).toEqual({ x: 10, y: 20, w: 100, h: 50 });
    expect(ellipseRect({ x: 110, y: 70 }, { x: 10, y: 20 })).toEqual({ x: 10, y: 20, w: 100, h: 50 });
    expect(ellipseGeometry(op({ x: 10, y: 20 }, { x: 110, y: 70 }))).toEqual({ cx: 60, cy: 45, rx: 50, ry: 25 });
  });

  it('Shift makes it a circle: the longer side wins, the drag direction survives', () => {
    expect(ellipseRect({ x: 0, y: 0 }, { x: 100, y: 40 }, true)).toEqual({ x: 0, y: 0, w: 100, h: 100 });
    expect(ellipseRect({ x: 100, y: 100 }, { x: 0, y: 60 }, true)).toEqual({ x: 0, y: 0, w: 100, h: 100 });
    const geo = ellipseGeometry(op({ x: 50, y: 50 }, { x: 10, y: 90 }));
    expect(geo.rx).toBe(20);
    expect(geo.ry).toBe(20);
  });

  it('hit test: inside the ellipse, on its stroke band, outside', () => {
    const e = op({ x: 0, y: 0 }, { x: 200, y: 100 }); // centre 100,50 r 100,50
    expect(pointInEllipse(e, { x: 100, y: 50 })).toBe(true);
    expect(pointInEllipse(e, { x: 195, y: 50 })).toBe(true);
    expect(pointInEllipse(e, { x: 190, y: 10 })).toBe(false); // inside the bbox, outside the ellipse
    expect(pointInEllipse(e, { x: 250, y: 50 })).toBe(false);
    expect(pointOnEllipse(e, { x: 200, y: 50 }, 4)).toBe(true); // right vertex
    expect(pointOnEllipse(e, { x: 100, y: 0 }, 4)).toBe(true); // top vertex
    expect(pointOnEllipse(e, { x: 100, y: 50 }, 4)).toBe(false); // centre
    expect(pointOnEllipse(e, { x: 100, y: 5 }, 3)).toBe(false); // ~5px inside, band 3
  });

  it('picks the topmost ellipse under the pointer, skipping other shapes', () => {
    const ops: EditOp[] = [
      op({ x: 0, y: 0 }, { x: 200, y: 100 }),
      { kind: 'draw', color: '#fff', width: 2, points: [{ x: 200, y: 50 }] },
      op({ x: 0, y: 0 }, { x: 100, y: 50 }),
    ];
    expect(ellipseAt(ops, { x: 100, y: 25 }, 6)).toBe(2);
    expect(ellipseAt(ops, { x: 200, y: 50 }, 6)).toBe(0);
    expect(ellipseAt(ops, { x: 100, y: 50 }, 6)).toBeUndefined();
  });

  it('flattens into the output canvas as one stroked ellipse at the right scale', () => {
    const { ctx, calls } = recorder();
    // Flatten path (`ImageEditor.done`): image px offset by the crop, scale 1.
    drawMarks(ctx, [op({ x: 10, y: 20 }, { x: 110, y: 70 })], { x: 10, y: 20 }, 1);
    expect(calls).toEqual(['begin', 'e50,25,50,25,0,0,6.283185307179586', 'stroke']);
    // Editor preview path: origin = shown crop, scale = fitted box scale.
    calls.length = 0;
    drawMarks(ctx, [op({ x: 10, y: 20 }, { x: 110, y: 70 })], { x: 10, y: 20 }, 2);
    expect(calls[1]).toBe('e100,50,100,50,0,0,6.283185307179586');
  });

  it('a zero-size ellipse paints nothing (a tap is not a shape)', () => {
    const { ctx, calls } = recorder();
    drawMarks(ctx, [op({ x: 5, y: 5 }, { x: 5, y: 5 })]);
    expect(calls).toEqual([]);
  });
});
