/**
 * Pure queue helpers behind the task origin (OIN-40): the Back target of a
 * task detail and the snapshot / position / neighbour / leave arithmetic of a
 * Superagent queue. No state, no imports — phoneNav uses it too.
 */
export type QueueKind = 'review' | 'needs';
export type QueueRef = { project: string; task: string };

const sameRef = (a: QueueRef, b: QueueRef) => a.project === b.project && a.task === b.task;

/** Pure: the route Back returns to (phone Back table + the label of the header arrow). */
export function taskBackTarget(from?: string): '/superagent' | '/board' {
  return from === 'superagent' ? '/superagent' : '/board';
}

/** Pure: a queue snapshot — duplicates dropped, order kept, and the opened task always in it. */
export function snapshotQueue(items: readonly QueueRef[], current: QueueRef): QueueRef[] {
  const seen: QueueRef[] = [];
  for (const item of items) if (!seen.some((other) => sameRef(other, item))) seen.push(item);
  return seen.some((item) => sameRef(item, current)) ? seen : [current, ...seen];
}

/** Pure: 1-based position of the task in the queue (undefined: not in it). */
export function queuePosition(items: readonly QueueRef[], task: string): { index: number; total: number } | undefined {
  const at = items.findIndex((item) => item.task === task);
  return at < 0 ? undefined : { index: at + 1, total: items.length };
}

/** Pure: the neighbour `delta` steps away (no wrap). */
export function queueNeighbour(items: readonly QueueRef[], task: string, delta: -1 | 1): QueueRef | undefined {
  const at = items.findIndex((item) => item.task === task);
  return at < 0 ? undefined : items[at + delta];
}

/**
 * Pure: the queue once `task` has left it, and what to show next — the item
 * that slid into its place, else the nearest earlier one that is still
 * waiting. `next` undefined = the queue is exhausted.
 */
export function queueAfterLeave(items: readonly QueueRef[], task: string): { items: QueueRef[]; next?: QueueRef } {
  const at = items.findIndex((item) => item.task === task);
  if (at < 0) return { items: [...items], next: undefined };
  const rest = items.filter((_, i) => i !== at);
  return { items: rest, next: rest[at] ?? rest[rest.length - 1] };
}

