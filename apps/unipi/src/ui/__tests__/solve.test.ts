/**
 * Solver acceptance (ui-protocol §4.3, §13.7, Appendix B): every vector in
 * `crates/oino-ui/tests/layout_vectors.json` — generated from ratatui 0.29 by
 * `crates/oino-tui/tests/gen_layout_vectors.rs` — matches within ±1 cell at
 * unit scale.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { clearSolveCache, solveAxis, splitRect, type Rect } from '../layout/solve';
import type { Constraint, Direction, Flex, Margin } from '../types';

type Vector = {
  name: string;
  direction: Direction;
  size: number;
  constraints: Constraint[];
  flex: Flex;
  spacing: number;
  margin: Margin | null;
  expected: Array<[number, number, number, number]>;
};

const file = JSON.parse(readFileSync(resolve(__dirname, '../../../../../crates/oino-ui/tests/layout_vectors.json'), 'utf8')) as {
  ratatui: string;
  cross: number;
  vectors: Vector[];
};

function areaOf(v: Vector): Rect {
  return v.direction === 'Horizontal' ? { x: 0, y: 0, width: v.size, height: file.cross } : { x: 0, y: 0, width: file.cross, height: v.size };
}

describe('layout vectors (ratatui 0.29)', () => {
  it('covers the generated matrix', () => {
    expect(file.ratatui).toBe('0.29');
    expect(file.vectors.length).toBeGreaterThanOrEqual(200);
  });

  it.each(file.vectors.map((v) => [v.name, v] as const))('%s', (_name, v) => {
    const got = splitRect({ constraints: v.constraints, flex: v.flex, spacing: v.spacing, margin: v.margin, direction: v.direction }, areaOf(v));
    expect(got).toHaveLength(v.expected.length);
    got.forEach((r, i) => {
      const [x, y, w, h] = v.expected[i]!;
      const ctx = `${v.name} segment ${i}: got ${JSON.stringify(r)} want ${JSON.stringify(v.expected[i])}`;
      expect(Math.abs(r.x - x), ctx).toBeLessThanOrEqual(1);
      expect(Math.abs(r.y - y), ctx).toBeLessThanOrEqual(1);
      expect(Math.abs(r.width - w), ctx).toBeLessThanOrEqual(1);
      expect(Math.abs(r.height - h), ctx).toBeLessThanOrEqual(1);
    });
  });

  it('reports how many vectors match exactly (drift guard)', () => {
    let exact = 0;
    for (const v of file.vectors) {
      const got = splitRect({ constraints: v.constraints, flex: v.flex, spacing: v.spacing, margin: v.margin, direction: v.direction }, areaOf(v));
      if (got.every((r, i) => [r.x, r.y, r.width, r.height].join() === v.expected[i]!.join())) exact++;
    }
    // ±1 is the acceptance; the port is exact today — keep it that way.
    expect(exact).toBe(file.vectors.length);
  });
});

describe('solveAxis', () => {
  it('scales cell constraints to pixels (app units)', () => {
    clearSolveCache();
    // Length(3) rows of 20 px + Fill in a 200 px column.
    expect(solveAxis({ constraints: [{ Length: 3 }, { Fill: 1 }], scale: 20 }, 200)).toEqual([
      [0, 60],
      [60, 140],
    ]);
  });

  it('defaults unreadable constraints to Fill(1) and bare integers to Length', () => {
    expect(solveAxis({ constraints: [5, undefined] }, 20)).toEqual([
      [0, 5],
      [5, 15],
    ]);
  });

  it('memoizes identical inputs', () => {
    clearSolveCache();
    const a = solveAxis({ constraints: [{ Fill: 1 }, { Fill: 2 }] }, 90);
    expect(solveAxis({ constraints: [{ Fill: 1 }, { Fill: 2 }] }, 90)).toBe(a);
  });

  it('returns nothing for no constraints', () => {
    expect(solveAxis({ constraints: [] }, 50)).toEqual([]);
  });
});
