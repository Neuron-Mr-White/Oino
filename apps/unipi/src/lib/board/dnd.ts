/**
 * Drag & drop between / within lane columns (UNI-117 desktop + web).
 *
 * The kanboard web UI's Board.tsx drag & drop, re-done as pure helpers so
 * the rules are testable without a DOM, plus a tiny reactive drag state
 * the lane columns share. Only the desktop shell turns it on (pointer
 * devices); the phone keeps long-press → reorder buttons / move rows.
 *
 * Rules (identical to the web UI):
 *  - a running card (agent claim) can't be dragged — a toast says why;
 *    cards in final lanes (Done / Cancelled / Archive) can't be dragged;
 *  - the drop position snaps to the start of a same-lane dependency
 *    chain (a chain is drawn as one block and never split);
 *  - a lane the rules don't allow is faded while dragging and refuses
 *    the drop with "X can't move from A to B";
 *  - a move that needs a note opens the comment sheet first, then the
 *    order is applied;
 *  - dropping back where it was is a no-op.
 */
import { createRoot, createSignal } from 'solid-js';
import { canMove, needsComment, type Rules, type Task } from './api';
import type { LaneItem } from './view';
import type { OrderTarget } from './api';

export const FINAL_LANES = new Set(['done', 'cancelled', 'archived']);

/** Why a card can't be picked up (undefined = draggable). */
export function dragBlocked(task: Task): string | undefined {
  if (task.run) return `${task.id} is claimed by session ${task.run.session ?? '?'} — it moves to In Review when the agent finishes`;
  if (FINAL_LANES.has(task.status)) return `${task.id} is ${task.status === 'archived' ? 'archived' : task.status} — use its menu to archive it`;
  return undefined;
}

/**
 * Where in `items` (the lane as drawn, without the dragged card) a drop
 * at `index` lands: snaps back to the first member of the chain block
 * the index falls into. Returns the id to insert before (null = end).
 */
export function snapBefore(items: Array<Pick<LaneItem, 'chain'> & { task: { id: string } }>, index: number): string | null {
  if (index < 0 || index >= items.length) return null;
  let at = index;
  while (at > 0 && (items[at]!.chain === 'middle' || items[at]!.chain === 'last')) at -= 1;
  return items[at]!.task.id;
}

/**
 * The index of the first card whose vertical midpoint is below `y` (the
 * web's midpoint test); `rects` are the drawn cards without the dragged
 * one. -1 = after the last card.
 */
export function indexAt(rects: Array<{ top: number; height: number }>, y: number): number {
  return rects.findIndex((rect) => y < rect.top + rect.height / 2);
}

export type DropPlan =
  | { kind: 'none' }
  | { kind: 'refused'; message: string }
  /** Move to another lane (maybe a note first), then apply `order` unless the move is undoable. */
  | { kind: 'move'; to: string; note: string | null; order: OrderTarget | null }
  /** Same lane, new position. */
  | { kind: 'reorder'; order: OrderTarget };

/**
 * What a drop means. `laneIds` is the target lane's stored order
 * (unfiltered), `before` the id the card lands before (null = end).
 */
export function planDrop(rules: Rules, task: Task, lane: string, before: string | null, laneIds: string[], laneLabel: (id: string) => string): DropPlan {
  if (before === task.id) return { kind: 'none' };
  if (lane === task.status) {
    const index = laneIds.indexOf(task.id);
    const unchanged = before === null ? index === laneIds.length - 1 : laneIds[index + 1] === before;
    if (unchanged) return { kind: 'none' };
    return { kind: 'reorder', order: before ? { before } : { bottom: true } };
  }
  if (!canMove(rules, task, lane)) {
    return { kind: 'refused', message: `${task.id} can't move from ${laneLabel(task.status)} to ${laneLabel(lane)}` };
  }
  return {
    kind: 'move',
    to: lane,
    note: needsComment(rules, task.status, lane),
    order: before ? { before } : { bottom: true },
  };
}

/** Is this lane a valid drop target for the dragged card? (fades the others) */
export function dropAllowed(rules: Rules, task: Task, lane: string): boolean {
  return lane === task.status || canMove(rules, task, lane);
}

// ─── shared drag state (one drag at a time) ─────────────────────────────────

export type DragState = { task: Task; lane: string | null; before: string | null };

function createDrag() {
  const [drag, setDrag] = createSignal<DragState | null>(null);
  return {
    drag,
    start: (task: Task) => setDrag({ task, lane: null, before: null }),
    over: (lane: string | null, before: string | null) =>
      setDrag((current) => (current && (current.lane !== lane || current.before !== before) ? { ...current, lane, before } : current)),
    end: () => setDrag(null),
  };
}

let shared: ReturnType<typeof createDrag> | undefined;
export function boardDrag(): ReturnType<typeof createDrag> {
  if (!shared) shared = createRoot(() => createDrag());
  return shared;
}
export function resetBoardDrag(): void {
  shared = undefined;
}

/** dataTransfer type for an in-app card drag (so OS file drops are told apart). */
export const CARD_MIME = 'application/x-unipi-task';
