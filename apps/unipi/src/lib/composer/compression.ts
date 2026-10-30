/**
 * The composer toolbar's narrow-width rule (WP-A13, Z-64).
 *
 * ZCode takes the seven cumulative compression steps from
 * `docs/superpowers/specs/2026-08-18-cua-composer-entry-design.md`:
 *
 *   1 Computer Use square → 2 mode chip square → 3 Plan square →
 *   4 think icon + level bar → 5 hide provider text → 6 think square →
 *   7 model square
 *
 * The bar measures its real widths, takes the **smallest** step whose total
 * fits and stops there ("每档够用即停"); widening restores in reverse order
 * because the step only ever goes down again. A control the bar does not
 * have contributes no width at any step, so its step is skipped.
 *
 * Pure: the widths come from the DOM, the decision is here and is tested at
 * fixed widths.
 */

/** Controls that can fold into a square (or a bar-only chip) for the bar. */
export type ToolbarSlot = 'computer' | 'permission' | 'plan' | 'think' | 'model';

/** One measured toolbar item; `slot` marks the compressible ones. */
export type ToolbarItem = { slot?: ToolbarSlot; width: number };

/** Square icon button size (ZCode: 28px; our `.tb-chip` square folds to 30 px —
 *  composer.css `width: 30px` — so the model must not under-budget it: an
 *  underestimated fold leaves the send chevron ~2 px over the edge on phones). */
export const COMPACT = 30;
/** Step 4's think form: the icon plus the small level bar, no text. */
export const THINK_LITE = 36;

/** The step at which each slot folds (ZCode's seven-step order). */
const FOLD_AT: Record<ToolbarSlot, number> = { computer: 1, permission: 2, plan: 3, think: 4, model: 7 };

/** A slot's width once `step` is applied (think has two folded forms). */
export function slotWidth(slot: ToolbarSlot, full: number, step: number): number {
  const fold = FOLD_AT[slot];
  if (step < fold) return full;
  if (slot === 'think') return step < 6 ? THINK_LITE : COMPACT;
  return COMPACT;
}

/** Total bar width at `step` (items + `gap` between each pair). */
export function widthAtStep(step: number, items: readonly ToolbarItem[], gap = 8): number {
  const content = items.reduce((sum, item) => sum + (item.slot ? slotWidth(item.slot, item.width, step) : item.width), 0);
  return content + gap * Math.max(0, items.length - 1);
}

/** Number of steps (7 = ZCode's seven-step order). */
export const MAX_STEP = 7;

/** The smallest step at which the bar fits `available` px; when even the last
 *  step is too wide (or nothing folds), the last step that actually narrows
 *  the bar — a step that changes nothing is never reported. */
export function compressionStep(available: number, items: readonly ToolbarItem[], gap = 8): number {
  let effective = 0;
  for (let step = 1; step <= MAX_STEP; step++) {
    if (widthAtStep(step, items, gap) < widthAtStep(step - 1, items, gap)) effective = step;
  }
  for (let step = 0; step <= MAX_STEP; step++) {
    if (widthAtStep(step, items, gap) <= available) return step;
  }
  return effective;
}

/** What one step hides, for the chip titles/aria labels in captures and tests. */
export const STEP_LABELS: readonly string[] = [
  'Full toolbar',
  'Computer use · icon',
  'Permission · icon',
  'Plan · icon',
  'Thinking · level bar',
  'Model · no provider',
  'Thinking · icon',
  'Model · icon',
];
