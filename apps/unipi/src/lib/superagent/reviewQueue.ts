/**
 * The Workbench Review queue's filter and sort (pure): which tickets show for the chosen
 * priorities and labels, and in what order. The default order is the host's own.
 */
import { PRIORITY_RANK } from '../board/api';
import type { WorkbenchSort } from './workbenchState';

/** What the queue knows of a ticket (the board's own priority and labels; empty until its detail has loaded). */
export type QueueMeta = { priority?: string; labels?: string[]; updated?: string } | undefined;
export type QueueRow = { projectId: string; taskId: string };

export const SORT_LABEL: Record<WorkbenchSort, string> = {
  default: 'Default order',
  priority: 'Priority',
  updated: 'Newest updated',
  waiting: 'Oldest waiting',
  id: 'Ticket id',
};

const rank = (m: QueueMeta) => PRIORITY_RANK[m?.priority || 'none'] ?? PRIORITY_RANK.none!;
const at = (m: QueueMeta) => Date.parse(m?.updated ?? '') || 0;
const byId = (a: QueueRow, b: QueueRow) => a.projectId.localeCompare(b.projectId) || a.taskId.localeCompare(b.taskId, undefined, { numeric: true });

/** Rows matching every chosen group: any of the priorities (empty: all) and any of the labels (empty: all). */
export function filterQueue<T extends QueueRow>(rows: readonly T[], meta: (r: T) => QueueMeta, priorities: readonly string[], labels: readonly string[]): T[] {
  if (!priorities.length && !labels.length) return [...rows];
  return rows.filter((r) => {
    const m = meta(r);
    return (!priorities.length || priorities.includes(m?.priority || 'none')) && (!labels.length || (m?.labels ?? []).some((l) => labels.includes(l)));
  });
}

/** A stable sort: ties keep the host's order. `waiting` is the longest since the ticket last changed (it reached review then). */
export function sortQueue<T extends QueueRow>(rows: readonly T[], meta: (r: T) => QueueMeta, sort: WorkbenchSort): T[] {
  const list = [...rows];
  if (sort === 'default') return list;
  const order = new Map(list.map((r, i) => [r, i] as const));
  const compare: Record<Exclude<WorkbenchSort, 'default'>, (a: T, b: T) => number> = {
    priority: (a, b) => rank(meta(a)) - rank(meta(b)),
    updated: (a, b) => at(meta(b)) - at(meta(a)),
    waiting: (a, b) => (at(meta(a)) || Infinity) - (at(meta(b)) || Infinity),
    id: byId,
  };
  const cmp = compare[sort];
  return list.sort((a, b) => cmp(a, b) || order.get(a)! - order.get(b)!);
}
