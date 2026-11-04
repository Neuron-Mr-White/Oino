/**
 * Toolbar compression (WP-A13 acceptance, Z-64): the seven-step order at six
 * widths — every step is taken on its own, in ZCode's order, restoring in
 * reverse, and a control the bar lacks is skipped instead of merging two
 * steps.
 */
import { describe, expect, it } from 'vitest';
import { COMPACT, compressionStep, itemWidth, MAX_STEP, slotWidth, THINK_LITE, widthAtStep, type ToolbarItem } from './compression';

/** A typical full width toolbar: + , permission, plan, spacer-fixed, think, model, context, send. */
const items: ToolbarItem[] = [
  { width: 32 }, // +
  { slot: 'permission', width: 84 },
  { slot: 'plan', width: 76 },
  { slot: 'think', width: 92 },
  { slot: 'model', width: 148 },
  { width: 30 }, // context ring
  { width: 40 }, // send
];
const GAP = 8;

describe('seven-step toolbar compression (Z-64)', () => {
  it('starts full and folds each control at its own step', () => {
    const full = widthAtStep(0, items, GAP);
    expect(compressionStep(full, items, GAP)).toBe(0);
    expect(slotWidth('permission', 84, 1)).toBe(84); // step 1 is Computer Use
    expect(slotWidth('permission', 84, 2)).toBe(COMPACT);
    expect(slotWidth('plan', 76, 2)).toBe(76);
    expect(slotWidth('plan', 76, 3)).toBe(COMPACT);
    expect(slotWidth('think', 92, 3)).toBe(92);
    expect(slotWidth('think', 92, 4)).toBe(THINK_LITE); // icon + level bar
    expect(slotWidth('think', 92, 5)).toBe(THINK_LITE); // step 5 = provider text
    expect(slotWidth('think', 92, 6)).toBe(COMPACT);
    expect(slotWidth('model', 148, 6)).toBe(148);
    expect(slotWidth('model', 148, 7)).toBe(COMPACT);
  });

  /** Steps this bar really has: 1 (Computer Use) and 5 (provider text) fold
   *  nothing here, so the six widths below are its six effective steps. */
  const EFFECTIVE = [0, 2, 3, 4, 6, 7];

  it('six widths land on six different steps, in order', () => {
    const at = (step: number) => widthAtStep(step, items, GAP);
    expect(EFFECTIVE.map((step) => compressionStep(at(step), items, GAP))).toEqual(EFFECTIVE);
    // One pixel narrower moves to the next effective step (2 → 3 here).
    expect(compressionStep(at(2) - 1, items, GAP)).toBe(3);
    expect(compressionStep(at(3) - 1, items, GAP)).toBe(4);
  });

  it('never exceeds the last step that changes anything', () => {
    expect(compressionStep(0, items, GAP)).toBe(MAX_STEP);
    expect(compressionStep(-500, items, GAP)).toBe(MAX_STEP);
    expect(widthAtStep(MAX_STEP, items, GAP)).toBeLessThan(widthAtStep(0, items, GAP));
  });

  it('skips a control the bar does not have (no merged steps)', () => {
    const noThink = items.filter((i) => i.slot !== 'think');
    // Without a think chip, step 4 and 5 change nothing: the same width picks step 3.
    expect(widthAtStep(4, noThink, GAP)).toBe(widthAtStep(3, noThink, GAP));
    expect(compressionStep(widthAtStep(3, noThink, GAP), noThink, GAP)).toBe(3);
    // A step that folds nothing is never chosen over an earlier fitting one.
    expect(compressionStep(widthAtStep(6, noThink, GAP), noThink, GAP)).toBeLessThanOrEqual(4);
  });

  it('restores in reverse: one more pixel of room goes back exactly one step', () => {
    const at = (step: number) => widthAtStep(step, items, GAP);
    for (const step of EFFECTIVE.slice(1)) {
      // at(step) fits `step`; one more px also fits `step`, while at(step-1) does not.
      expect(compressionStep(at(step), items, GAP)).toBe(step);
      expect(compressionStep(at(step) + 1, items, GAP)).toBe(step);
      if (step < MAX_STEP) expect(compressionStep(at(step) - 1, items, GAP)).toBeGreaterThan(step);
    }
  });

  it('a bar with no compressible control always stays at step 0', () => {
    const plain = [{ width: 32 }, { width: 40 }];
    expect(compressionStep(72 + GAP, plain, GAP)).toBe(0);
    expect(compressionStep(0, plain, GAP)).toBe(0);
  });
});

/** Owner 2026-11-04: the model capsule (context ring · name) is the flexible slot. */
describe('the flexible model capsule', () => {
  const capsule: ToolbarItem = { slot: 'model', width: 300, min: 148, compact: 54 };
  const row: ToolbarItem[] = [{ width: 32 }, { slot: 'permission', width: 84 }, { slot: 'plan', width: 76 }, { slot: 'think', width: 92 }, capsule, { width: 52 }];

  it('counts at its floor until it folds, then at its measured folded width', () => {
    expect(itemWidth(capsule, 0)).toBe(148);
    expect(itemWidth(capsule, 6)).toBe(148);
    expect(itemWidth(capsule, 7)).toBe(54);
    // A short name under the floor counts at its own width.
    expect(itemWidth({ ...capsule, width: 110 }, 0)).toBe(110);
  });

  it('gives up name length before any other chip folds', () => {
    const floorRow = widthAtStep(0, row, GAP);
    // Far less than the natural row, but the floor still fits: nothing folds.
    expect(floorRow).toBeLessThan(32 + 84 + 76 + 92 + 300 + 52);
    expect(compressionStep(floorRow, row, GAP)).toBe(0);
    expect(compressionStep(floorRow - 1, row, GAP)).toBe(2);
  });

  it('a folded slot uses its measured width, not the 30 px guess', () => {
    const phone: ToolbarItem[] = [{ width: 48 }, { slot: 'permission', width: 72, compact: 65 }, capsule, { width: 86 }];
    expect(widthAtStep(2, phone, 4)).toBe(48 + 65 + 148 + 86 + 3 * 4);
  });

  it('an empty slot (not offered) adds no gap', () => {
    const withEmpty: ToolbarItem[] = [{ width: 48 }, { slot: 'permission', width: 0 }, capsule, { width: 86 }];
    expect(widthAtStep(0, withEmpty, 4)).toBe(48 + 148 + 86 + 2 * 4);
  });
});
