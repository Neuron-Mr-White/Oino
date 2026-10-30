/**
 * Layout solver (ui-protocol §4.3): a port of ratatui 0.29's
 * `Layout::try_split` constraint formulation (`layout/layout.rs`:
 * `configure_area`, `configure_variable_in_area_constraints`,
 * `configure_variable_constraints`, `configure_flex_constraints`,
 * `configure_constraints`, `configure_fill_constraints`, `ALL_SEGMENT_GROW`,
 * `changes_to_rects`) onto `@lume/kiwi`, the JS port of the same Cassowary
 * solver ratatui uses. Checked against `crates/oino-ui/tests/layout_vectors.json`
 * (generated from ratatui itself) within ±1 cell.
 *
 * Units: at `scale = 1` one unit is one cell, exactly like the TUI. The app
 * solves in pixels: `scale` = px per unit (`--ui-col` / `--ui-row`), so cell
 * constraints (`Length`, `Min`, `Max`, spacing) become pixel sizes and the
 * result is rounded to whole pixels. Results are memoized (LRU 500, like
 * ratatui's layout cache).
 */
import { Constraint as KConstraint, Expression, Operator, Solver, Strength, Variable } from '@lume/kiwi';
import type { Constraint, Direction, Flex, Margin } from '../types';

/** ratatui `FLOAT_PRECISION_MULTIPLIER`. */
const FPM = 100;

// Cassowary strengths, as ratatui's `strengths` module derives them.
const REQUIRED = Strength.required;
const STRONG = Strength.strong;
const MEDIUM = Strength.medium;
const WEAK = Strength.weak;
const SPACER_SIZE_EQ = REQUIRED / 10;
const MIN_SIZE_GE = STRONG * 100;
const MAX_SIZE_LE = STRONG * 100;
const LENGTH_SIZE_EQ = STRONG * 10;
const PERCENTAGE_SIZE_EQ = STRONG;
const RATIO_SIZE_EQ = STRONG / 10;
const MIN_SIZE_EQ = MEDIUM * 10;
const MAX_SIZE_EQ = MEDIUM * 10;
const FILL_GROW = MEDIUM;
const GROW = MEDIUM / 10;
const SPACE_GROW = WEAK * 10;
const ALL_SEGMENT_GROW = WEAK;
/** `Element::is_empty` strength (`REQUIRED - 1`). */
const EMPTY_EQ = REQUIRED - 1;

type Norm =
  | { k: 'Length'; n: number }
  | { k: 'Percentage'; n: number }
  | { k: 'Ratio'; a: number; b: number }
  | { k: 'Min'; n: number }
  | { k: 'Max'; n: number }
  | { k: 'Fill'; n: number };

/** Normalizes the serde forms (`3` ≡ `{"Length":3}`); anything unreadable is `Fill(1)` (§4.3 default). */
export function normConstraint(c: Constraint | undefined): Norm {
  if (typeof c === 'number') return { k: 'Length', n: Math.max(0, c) };
  if (c && typeof c === 'object') {
    if ('Length' in c) return { k: 'Length', n: num(c.Length) };
    if ('Percentage' in c) return { k: 'Percentage', n: num(c.Percentage) };
    if ('Ratio' in c && Array.isArray(c.Ratio)) return { k: 'Ratio', a: num(c.Ratio[0]), b: num(c.Ratio[1]) };
    if ('Min' in c) return { k: 'Min', n: num(c.Min) };
    if ('Max' in c) return { k: 'Max', n: num(c.Max) };
    if ('Fill' in c) return { k: 'Fill', n: num(c.Fill) };
  }
  return { k: 'Fill', n: 1 };
}

function num(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) ? Math.max(0, v) : 0;
}

/** Margin on each axis, in units. */
export function marginOf(m: Margin | undefined | null): { horizontal: number; vertical: number } {
  if (typeof m === 'number') return { horizontal: Math.max(0, m), vertical: Math.max(0, m) };
  if (m && typeof m === 'object') return { horizontal: num(m.horizontal), vertical: num(m.vertical) };
  return { horizontal: 0, vertical: 0 };
}

export type AxisSpec = {
  constraints: readonly (Constraint | undefined)[];
  flex?: Flex;
  /** Gap between segments, in units. */
  spacing?: number;
  /** px per unit (1 = cells). */
  scale?: number;
};

/** One solved segment along the main axis: `[start, size]` relative to the axis start. */
export type Segment = [number, number];

const cache = new Map<string, Segment[]>();
const CACHE_MAX = 500;

/**
 * Splits a main axis of `size` (px at `scale`, cells at scale 1) into one
 * segment per constraint. Pure and memoized.
 */
export function solveAxis(spec: AxisSpec, size: number): Segment[] {
  const flex = spec.flex ?? 'Start';
  const spacing = Math.max(0, spec.spacing ?? 0);
  const scale = spec.scale && spec.scale > 0 ? spec.scale : 1;
  const key = `${flex}|${spacing}|${scale}|${size}|${JSON.stringify(spec.constraints)}`;
  const hit = cache.get(key);
  if (hit) {
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  const cs = spec.constraints.map(normConstraint);
  const spacingPx = spacing * scale;
  const out = solveFast(cs, flex, spacingPx, scale, Math.max(0, size)) ?? solveUncached(cs, flex, spacingPx, scale, Math.max(0, size));
  if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value as string);
  cache.set(key, out);
  return out;
}

/** Clears the memo (tests, benches). */
export function clearSolveCache(): void {
  cache.clear();
}

type El = { s: Variable; e: Variable };
const sizeOf = (el: El) => new Expression(el.e, [-1, el.s]);

/** Fill weight floor, mirroring the kiwi path's `Math.max(c.n, 1e-6)`. */
const growW = (n: number) => Math.max(n, 1e-6);

/**
 * Closed form for the dominant case — `flex: "Start"`, constraints from
 * {Length, Percentage, Fill} only, and the constraints fit the axis — the
 * §13.5 solve budget (≤ 0.2 ms for 16 children) is unreachable through a
 * cold Cassowary solve, and this case needs no arbitration: spacing is
 * satisfied first (SPACER_SIZE_EQ > LENGTH_SIZE_EQ), fixed and percentage
 * segments take their sizes, fills share the remainder by weight, and the
 * trailing spacer absorbs any slack. Returns `null` for anything the solver
 * must decide (Min/Max/Ratio, other flex, over-constrained) so the kiwi
 * formulation stays the single authority for those.
 */
function solveFast(cs: Norm[], flex: Flex, spacingPx: number, scale: number, size: number): Segment[] | null {
  if (flex !== 'Start' || cs.length === 0) return null;
  let fixed = 0;
  let pct = 0;
  let fills: number[] = [];
  for (const c of cs) {
    if (c.k === 'Length') fixed += c.n * scale;
    else if (c.k === 'Percentage') pct += (c.n / 100) * size;
    else if (c.k === 'Fill') fills.push(growW(c.n));
    else return null;
  }
  const usable = size - spacingPx * (cs.length - 1);
  const rest = usable - fixed - pct;
  if (rest < -1e-9) return null; // over-constrained: let the solver relax strengths
  const totalW = fills.reduce((a, b) => a + b, 0);
  if (fills.length > 0 && totalW <= 0) return null;
  let x = 0;
  const segs: Segment[] = [];
  for (const c of cs) {
    let s: number;
    if (c.k === 'Length') s = c.n * scale;
    else if (c.k === 'Percentage') s = (c.n / 100) * size;
    else s = (rest * (fills.shift() ?? 0)) / (totalW || 1);
    const start = Math.round(x);
    segs.push([start, Math.max(0, Math.round(x + s) - start)]);
    x += s + spacingPx;
  }
  return segs;
}

function solveUncached(cs: Norm[], flex: Flex, spacingPx: number, scale: number, size: number): Segment[] {
  if (cs.length === 0) return [];
  const solver = new Solver();
  const add = (lhs: Expression | Variable, op: Operator, rhs: Expression | Variable | number, strength: number) =>
    solver.addConstraint(new KConstraint(lhs, op, rhs, strength));

  const areaStart = 0;
  const areaEnd = size * FPM;
  const vars = Array.from({ length: cs.length * 2 + 2 }, () => new Variable());
  const spacers: El[] = [];
  for (let i = 0; i + 1 < vars.length; i += 2) spacers.push({ s: vars[i]!, e: vars[i + 1]! });
  const segments: El[] = [];
  for (let i = 1; i + 1 < vars.length; i += 2) segments.push({ s: vars[i]!, e: vars[i + 1]! });
  const area: El = { s: vars[0]!, e: vars[vars.length - 1]! };
  const areaSize = sizeOf(area);
  const legacy = flex === 'Legacy';

  // configure_area
  add(area.s, Operator.Eq, areaStart, REQUIRED);
  add(area.e, Operator.Eq, areaEnd, REQUIRED);
  // configure_variable_in_area_constraints
  for (const v of vars) {
    add(v, Operator.Ge, area.s, REQUIRED);
    add(v, Operator.Le, area.e, REQUIRED);
  }
  // configure_variable_constraints: each element's start ≤ its end (pairs after the first var)
  for (let i = 1; i + 1 < vars.length; i += 2) add(vars[i]!, Operator.Le, vars[i + 1]!, REQUIRED);

  // configure_flex_constraints
  const inner = spacers.slice(1, -1);
  const first = spacers[0]!;
  const last = spacers[spacers.length - 1]!;
  const spacingF = spacingPx * FPM;
  const isEmpty = (el: El) => add(sizeOf(el), Operator.Eq, 0, EMPTY_EQ);
  switch (flex) {
    case 'Legacy':
      for (const sp of inner) add(sizeOf(sp), Operator.Eq, spacingF, SPACER_SIZE_EQ);
      isEmpty(first);
      isEmpty(last);
      break;
    case 'SpaceAround':
      for (let i = 0; i < spacers.length; i++)
        for (let j = i + 1; j < spacers.length; j++) add(sizeOf(spacers[i]!), Operator.Eq, sizeOf(spacers[j]!), SPACER_SIZE_EQ);
      for (const sp of spacers) {
        add(sizeOf(sp), Operator.Ge, spacingF, SPACER_SIZE_EQ);
        add(sizeOf(sp), Operator.Eq, areaSize, SPACE_GROW);
      }
      break;
    case 'SpaceBetween':
      for (let i = 0; i < inner.length; i++)
        for (let j = i + 1; j < inner.length; j++) add(sizeOf(inner[i]!), Operator.Eq, sizeOf(inner[j]!), SPACER_SIZE_EQ);
      for (const sp of inner) {
        add(sizeOf(sp), Operator.Ge, spacingF, SPACER_SIZE_EQ);
        add(sizeOf(sp), Operator.Eq, areaSize, SPACE_GROW);
      }
      isEmpty(first);
      isEmpty(last);
      break;
    case 'Center':
      for (const sp of inner) add(sizeOf(sp), Operator.Eq, spacingF, SPACER_SIZE_EQ);
      add(sizeOf(first), Operator.Eq, areaSize, GROW);
      add(sizeOf(last), Operator.Eq, areaSize, GROW);
      add(sizeOf(first), Operator.Eq, sizeOf(last), SPACER_SIZE_EQ);
      break;
    case 'End':
      for (const sp of inner) add(sizeOf(sp), Operator.Eq, spacingF, SPACER_SIZE_EQ);
      isEmpty(last);
      add(sizeOf(first), Operator.Eq, areaSize, GROW);
      break;
    case 'Start':
    default:
      for (const sp of inner) add(sizeOf(sp), Operator.Eq, spacingF, SPACER_SIZE_EQ);
      isEmpty(first);
      add(sizeOf(last), Operator.Eq, areaSize, GROW);
      break;
  }

  // configure_constraints
  cs.forEach((c, i) => {
    const seg = sizeOf(segments[i]!);
    switch (c.k) {
      case 'Max':
        add(seg, Operator.Le, c.n * scale * FPM, MAX_SIZE_LE);
        add(seg, Operator.Eq, c.n * scale * FPM, MAX_SIZE_EQ);
        break;
      case 'Min':
        add(seg, Operator.Ge, c.n * scale * FPM, MIN_SIZE_GE);
        if (legacy) add(seg, Operator.Eq, c.n * scale * FPM, MIN_SIZE_EQ);
        else add(seg, Operator.Eq, areaSize, FILL_GROW);
        break;
      case 'Length':
        add(seg, Operator.Eq, c.n * scale * FPM, LENGTH_SIZE_EQ);
        break;
      case 'Percentage':
        add(seg, Operator.Eq, areaSize.multiply(c.n / 100), PERCENTAGE_SIZE_EQ);
        break;
      case 'Ratio':
        add(seg, Operator.Eq, areaSize.multiply(c.a / Math.max(c.b, 1)), RATIO_SIZE_EQ);
        break;
      case 'Fill':
        add(seg, Operator.Eq, areaSize, FILL_GROW);
        break;
    }
  });

  // configure_fill_constraints: Fill (and non-Legacy Min) segments share excess by weight.
  const growers: Array<{ w: number; el: El }> = [];
  cs.forEach((c, i) => {
    if (c.k === 'Fill') growers.push({ w: Math.max(c.n, 1e-6), el: segments[i]! });
    else if (!legacy && c.k === 'Min') growers.push({ w: 1, el: segments[i]! });
  });
  for (let i = 0; i < growers.length; i++)
    for (let j = i + 1; j < growers.length; j++) {
      const l = growers[i]!;
      const r = growers[j]!;
      add(sizeOf(l.el).multiply(r.w), Operator.Eq, sizeOf(r.el).multiply(l.w), GROW);
    }

  if (!legacy)
    for (let i = 0; i + 1 < segments.length; i++) add(sizeOf(segments[i]!), Operator.Eq, sizeOf(segments[i + 1]!), ALL_SEGMENT_GROW);

  solver.updateVariables();
  // changes_to_rects
  return segments.map((el) => {
    const start = Math.round(Math.round(el.s.value()) / FPM);
    const end = Math.round(Math.round(el.e.value()) / FPM);
    return [start, Math.max(0, end - start)] as Segment;
  });
}

export type Rect = { x: number; y: number; width: number; height: number };

export type LayoutSpec = AxisSpec & { direction?: Direction; margin?: Margin | null };

/**
 * ratatui `Layout::split(area)`: margin inset (`Rect::inner` — zero when the
 * margin doesn't fit), then the main-axis solve. Used by the vector tests and
 * by `Layout` in definite mode.
 */
export function splitRect(spec: LayoutSpec, area: Rect): Rect[] {
  const m = marginOf(spec.margin);
  const scale = spec.scale ?? 1;
  const mh = m.horizontal * scale;
  const mv = m.vertical * scale;
  const fits = area.width >= mh * 2 && area.height >= mv * 2;
  const inner: Rect = fits
    ? { x: area.x + mh, y: area.y + mv, width: area.width - mh * 2, height: area.height - mv * 2 }
    : { x: 0, y: 0, width: 0, height: 0 };
  const horizontal = spec.direction === 'Horizontal';
  const segs = solveAxis(spec, horizontal ? inner.width : inner.height);
  const base = horizontal ? inner.x : inner.y;
  return segs.map(([start, size]) =>
    horizontal
      ? { x: base + start, y: inner.y, width: size, height: inner.height }
      : { x: inner.x, y: base + start, width: inner.width, height: size },
  );
}
