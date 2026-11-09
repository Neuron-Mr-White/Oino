/**
 * Where an open task detail came from, and the queue it was opened out of
 * (OIN-40). The Superagent page opens a ticket over itself and remembers the
 * queue it was in (Review or Needs you) as a snapshot of the list at open
 * time. Back then leaves the detail and lands on the page underneath — never
 * on the Kanboard — Prev/Next move inside the snapshot, and Mark done /
 * Rework / Unblock move on to the next item. A task opened from the board has
 * no origin and behaves as before.
 *
 * Explicit state, not router history: the detail is an overlay, moving
 * between tasks only swaps the shown id (one Back entry for the whole walk).
 */
import { createSignal } from 'solid-js';
import { boardStore } from './store';
import { boardUi, rememberLast } from './ui';
import { queueAfterLeave, queueNeighbour, queuePosition, snapshotQueue, type QueueKind, type QueueRef } from './taskQueue';

export type { QueueKind, QueueRef } from './taskQueue';

export type TaskOrigin = { from: 'superagent'; queue: QueueKind; hostId: string; items: QueueRef[] };

const sameRef = (a: QueueRef, b: QueueRef) => a.project === b.project && a.task === b.task;

const [origin, setOrigin] = createSignal<TaskOrigin | undefined>();
/** True while a hop to a task of another project loads that board. */
const [switching, setSwitching] = createSignal(false);
let hop = 0;

export const taskOrigin = origin;
export const taskOriginSwitching = switching;

/** The current task's place in its queue (reactive): `3 of 12`, and whether Prev/Next exist. */
export function queueView(task: () => string | undefined) {
  const at = () => {
    const o = origin();
    const id = task();
    return o && id ? queuePosition(o.items, id) : undefined;
  };
  return {
    position: at,
    queue: () => origin()?.queue,
    hasPrev: () => { const p = at(); return !!p && p.index > 1; },
    hasNext: () => { const p = at(); return !!p && p.index < p.total; },
  };
}

async function show(o: TaskOrigin, ref: QueueRef): Promise<void> {
  const mine = ++hop;
  const store = boardStore();
  const at = store.target();
  if (at?.hostId !== o.hostId || at.slug !== ref.project) {
    setSwitching(true);
    try {
      rememberLast(o.hostId, ref.project);
      await store.open(o.hostId, ref.project);
    } finally {
      if (mine === hop) setSwitching(false);
    }
  }
  if (mine !== hop || origin() !== o) return;
  // Replace, never stack: one Back leaves the detail whichever task is shown.
  boardUi().openTask(ref.task);
}

/** Opens `current` from a Superagent queue; `items` is the list as the owner saw it. */
export async function openFromSuperagent(hostId: string, queue: QueueKind, items: readonly QueueRef[], current: QueueRef): Promise<void> {
  const o: TaskOrigin = { from: 'superagent', queue, hostId, items: snapshotQueue(items, current) };
  setOrigin(o);
  await show(o, current);
}

/** Prev (-1) / Next (+1) inside the queue. */
export async function stepQueue(delta: -1 | 1): Promise<void> {
  const o = origin();
  const id = boardUi().openedTask();
  if (!o || !id) return;
  const to = queueNeighbour(o.items, id, delta);
  if (to) await show(o, to);
}

/**
 * The shown task left its lane (Mark done, Rework, Unblock): drop it from the
 * queue and show the next one. With nothing left the detail closes — the
 * Superagent page underneath is the "queue done" state. Returns whether the
 * queue is exhausted.
 */
export async function advanceQueue(task: string): Promise<boolean> {
  const o = origin();
  if (!o) return false;
  const out = queueAfterLeave(o.items, task);
  const next: TaskOrigin = { ...o, items: out.items };
  setOrigin(next);
  if (!out.next) {
    boardUi().closeTask();
    boardUi().toast(`${o.queue === 'review' ? 'Review' : 'Needs you'} queue done`, 'success');
    return true;
  }
  await show(next, out.next);
  return false;
}

/**
 * Live pruning: items that left the Superagent list on their own (the lead
 * moved them, another device reviewed them) drop out of the queue. The shown
 * task stays until the owner leaves it; nothing is ever added to a snapshot.
 */
export function pruneQueue(live: readonly QueueRef[]): void {
  const o = origin();
  if (!o) return;
  const shown = boardUi().openedTask();
  const items = o.items.filter((item) => item.task === shown || live.some((l) => sameRef(l, item)));
  if (items.length !== o.items.length) setOrigin({ ...o, items });
}

export function endQueue(): void {
  hop += 1;
  setSwitching(false);
  setOrigin(undefined);
}
