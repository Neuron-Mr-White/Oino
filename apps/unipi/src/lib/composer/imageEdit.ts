/**
 * The composer image editor's model (WP-A13, moved from `lib/imageEdit.ts`):
 * pure, so it is tested without a canvas. An edit is a list of operations
 * over the ORIGINAL image's pixel space: strokes, arrows and ellipses are
 * drawn in image coordinates, crop is a rectangle in image coordinates
 * applied last (so drawing after cropping still lands where you see it, and
 * undo just pops an operation).
 *
 * D-5 adds the **ellipse** shape: dragged as a bounding box (`from`/`to` are
 * opposite corners), Shift constrains it to a circle, it uses the same
 * stroke colour/width as the other shapes, and it is drawn/cropped/flattened
 * like them. `ellipseRect` is the bbox, `pointOnEllipse`/`ellipseAt` are the
 * hit test the editor uses to drag an existing ellipse (the same way the
 * crop frame can be moved).
 */

export type Point = { x: number; y: number };
export type Stroke = { kind: 'draw'; color: string; width: number; points: Point[] };
export type Arrow = { kind: 'arrow'; color: string; width: number; from: Point; to: Point };
export type Ellipse = { kind: 'ellipse'; color: string; width: number; from: Point; to: Point };
export type Crop = { kind: 'crop'; rect: Rect };
export type EditOp = Stroke | Arrow | Ellipse | Crop;
export type Rect = { x: number; y: number; w: number; h: number };

export const COLORS = ['#F07818', '#E5484D', '#F5D90A', '#30A46C', '#3E8EF7', '#FFFFFF', '#111111'] as const;
/** Stroke widths as a fraction of the image's shorter side — the same look on any size of image. */
export const WIDTHS = [0.006, 0.012, 0.024] as const;

/** The crop in effect: the last crop op, else the whole image. */
export function currentCrop(ops: readonly EditOp[], width: number, height: number): Rect {
  for (let i = ops.length - 1; i >= 0; i--) {
    const op = ops[i]!;
    if (op.kind === 'crop') return op.rect;
  }
  return { x: 0, y: 0, w: width, h: height };
}

/** Keep a crop rectangle inside the image and at least `min` px each way. */
export function clampRect(r: Rect, width: number, height: number, min = 16): Rect {
  let x = Math.min(r.x, r.x + r.w);
  let y = Math.min(r.y, r.y + r.h);
  let w = Math.abs(r.w);
  let h = Math.abs(r.h);
  x = Math.max(0, Math.min(x, width - min));
  y = Math.max(0, Math.min(y, height - min));
  w = Math.max(min, Math.min(w, width - x));
  h = Math.max(min, Math.min(h, height - y));
  return { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) };
}

/** Map a point on the displayed (fitted, cropped) image back to original image pixels. */
export function toImage(p: Point, box: { left: number; top: number; scale: number }, crop: Rect): Point {
  return { x: crop.x + (p.x - box.left) / box.scale, y: crop.y + (p.y - box.top) / box.scale };
}

/** Fit `crop` (w×h) into a viewport, centred: where to draw and at what scale. */
export function fitBox(crop: Rect, viewW: number, viewH: number, pad = 0): { left: number; top: number; scale: number; w: number; h: number } {
  const scale = Math.min((viewW - pad * 2) / crop.w, (viewH - pad * 2) / crop.h);
  const w = crop.w * scale;
  const h = crop.h * scale;
  return { left: (viewW - w) / 2, top: (viewH - h) / 2, scale, w, h };
}

/** The bounding box an ellipse drag spans: opposite corners `from`/`to`,
 * normalised; `square` (Shift) makes it a circle — the longer side wins so a
 * circle never shrinks under the pointer. */
export function ellipseRect(from: Point, to: Point, square = false): Rect {
  let dx = to.x - from.x;
  let dy = to.y - from.y;
  if (square) {
    const side = Math.max(Math.abs(dx), Math.abs(dy));
    dx = (dx < 0 ? -1 : 1) * side;
    dy = (dy < 0 ? -1 : 1) * side;
  }
  return { x: Math.min(from.x, from.x + dx), y: Math.min(from.y, from.y + dy), w: Math.abs(dx), h: Math.abs(dy) };
}

/** An ellipse's centre and radii in image pixels. */
export function ellipseGeometry(op: Pick<Ellipse, 'from' | 'to'>): { cx: number; cy: number; rx: number; ry: number } {
  const r = ellipseRect(op.from, op.to);
  return { cx: r.x + r.w / 2, cy: r.y + r.h / 2, rx: r.w / 2, ry: r.h / 2 };
}

/** Is `p` inside the ellipse (the hit test for grabbing one to drag)? */
export function pointInEllipse(op: Pick<Ellipse, 'from' | 'to'>, p: Point): boolean {
  const { cx, cy, rx, ry } = ellipseGeometry(op);
  if (rx <= 0 || ry <= 0) return false;
  const nx = (p.x - cx) / rx;
  const ny = (p.y - cy) / ry;
  return nx * nx + ny * ny <= 1;
}

/** Is `p` on the ellipse's stroke band (`tol` image px)? */
export function pointOnEllipse(op: Pick<Ellipse, 'from' | 'to'>, p: Point, tol: number): boolean {
  const { cx, cy, rx, ry } = ellipseGeometry(op);
  if (rx <= 0 || ry <= 0) return false;
  const nx = (p.x - cx) / rx;
  const ny = (p.y - cy) / ry;
  // Distance to the outline ≈ how far the normalised radius is from 1,
  // scaled back by the smaller radius (a good enough band test for a pointer).
  return Math.abs(Math.hypot(nx, ny) - 1) * Math.min(rx, ry) <= Math.max(tol, 1);
}

/** The topmost ellipse under `p` (stroke band `tol` wide), as its op index; a
 * tap inside a big ellipse's outline grabs nothing, so a new shape can be
 * drawn there. */
export function ellipseAt(ops: readonly EditOp[], p: Point, tol: number): number | undefined {
  for (let i = ops.length - 1; i >= 0; i--) {
    const op = ops[i]!;
    if (op.kind === 'ellipse' && pointOnEllipse(op, p, tol)) return i;
  }
  return undefined;
}

/** Arrow head points for a line from→to with stroke width `w`. */
export function arrowHead(from: Point, to: Point, w: number): [Point, Point] {
  const angle = Math.atan2(to.y - from.y, to.x - from.x);
  const len = Math.max(w * 4, 14);
  const spread = Math.PI / 7;
  return [
    { x: to.x - len * Math.cos(angle - spread), y: to.y - len * Math.sin(angle - spread) },
    { x: to.x - len * Math.cos(angle + spread), y: to.y - len * Math.sin(angle + spread) },
  ];
}

type Ctx = Pick<CanvasRenderingContext2D, 'beginPath' | 'moveTo' | 'lineTo' | 'stroke' | 'fill' | 'closePath' | 'ellipse'> & {
  strokeStyle: string | CanvasGradient | CanvasPattern;
  fillStyle: string | CanvasGradient | CanvasPattern;
  lineWidth: number;
  lineCap: CanvasLineCap;
  lineJoin: CanvasLineJoin;
};

/** Draw the marks (not the crop) in image coordinates, offset by `-origin` and scaled by `scale`. */
export function drawMarks(ctx: Ctx, ops: readonly EditOp[], origin: Point = { x: 0, y: 0 }, scale = 1): void {
  const X = (p: Point) => (p.x - origin.x) * scale;
  const Y = (p: Point) => (p.y - origin.y) * scale;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  for (const op of ops) {
    if (op.kind === 'crop') continue;
    ctx.strokeStyle = op.color;
    ctx.fillStyle = op.color;
    ctx.lineWidth = op.width * scale;
    if (op.kind === 'draw') {
      if (!op.points.length) continue;
      ctx.beginPath();
      ctx.moveTo(X(op.points[0]!), Y(op.points[0]!));
      // A tap is a dot.
      if (op.points.length === 1) ctx.lineTo(X(op.points[0]!) + 0.01, Y(op.points[0]!));
      for (const p of op.points.slice(1)) ctx.lineTo(X(p), Y(p));
      ctx.stroke();
    } else if (op.kind === 'ellipse') {
      const { cx, cy, rx, ry } = ellipseGeometry(op);
      if (rx <= 0 || ry <= 0) continue;
      ctx.beginPath();
      ctx.ellipse(X({ x: cx, y: cy }), Y({ x: cx, y: cy }), rx * scale, ry * scale, 0, 0, Math.PI * 2);
      ctx.stroke();
    } else {
      ctx.beginPath();
      ctx.moveTo(X(op.from), Y(op.from));
      ctx.lineTo(X(op.to), Y(op.to));
      ctx.stroke();
      const [a, b] = arrowHead(op.from, op.to, op.width);
      ctx.beginPath();
      ctx.moveTo(X(op.to), Y(op.to));
      ctx.lineTo(X(a), Y(a));
      ctx.lineTo(X(b), Y(b));
      ctx.closePath();
      ctx.fill();
    }
  }
}

/** Is there anything to save? (a crop equal to the whole image counts as nothing) */
export function hasEdits(ops: readonly EditOp[], width: number, height: number): boolean {
  return ops.some((op) => op.kind !== 'crop' || op.rect.x !== 0 || op.rect.y !== 0 || op.rect.w !== width || op.rect.h !== height);
}
