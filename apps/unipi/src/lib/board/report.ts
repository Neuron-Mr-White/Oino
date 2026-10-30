/**
 * Task detail helpers (UNI-131), ported from the kanboard web UI's
 * TaskPanel.tsx / Board.tsx: report labels in the activity timeline,
 * the status report behind In Review / Blocked, the move list the
 * status menu offers, the run (claim) state, and the reorder plan that
 * replaces drag & drop on the phone.
 */
import { LANES, type Rules, type Task } from './api';
import { laneLayout, type LaneItem } from './view';

/** Status-changing entries: a label chip, then the comment (the report). */
const REPORT_PREFIXES: Array<[RegExp, string]> = [
  [/^finished:\s*/, 'Finished'],
  [/^released to in_review:\s*/, 'Finished'],
  [/^blocked:\s*/, 'Blocked'],
  [/^released to blocked:\s*/, 'Blocked'],
  [/^unblocked:\s*/, 'Unblocked'],
  [/^rework:\s*/, 'Rework'],
  [/^released to todo:\s*/, 'Released'],
];

export function splitReport(text: string): { label: string | null; body: string } {
  for (const [pattern, label] of REPORT_PREFIXES) {
    if (pattern.test(text)) return { label, body: text.replace(pattern, '') };
  }
  return { label: null, body: text };
}

/** The report behind In Review / Blocked (older daemons: the blocked reason only). */
export function statusReport(task: Task): Task['statusReport'] {
  return (
    task.statusReport ??
    (task.status === 'blocked' && task.blockedReason?.text ? { kind: 'blocked' as const, ...task.blockedReason } : undefined)
  );
}

/** Lanes the status menu offers (web `moves()`): allowed, not the current one, not Archive (that's its own action). */
export function statusMoves(rules: Rules, task: Task): string[] {
  const explicit =
    Array.isArray(task.allowedMoves) && task.allowedMoves.length > 0 ? task.allowedMoves : (rules.allowedMoves[task.status] ?? []);
  const order = LANES.map((lane) => lane.id as string);
  return explicit
    .filter((status) => status !== task.status && status !== 'archived')
    .sort((a, b) => order.indexOf(a) - order.indexOf(b));
}

/**
 * Who may move the task right now:
 *  - `free`: no claim;
 *  - `running`: an agent session holds it (only the agent moves it on);
 *  - `stale`: the claiming process is dead on its host — the user may release it (needs a note);
 *  - `unknown`: claimed on another host, cannot be verified as dead.
 */
export type ClaimState = 'free' | 'running' | 'stale' | 'unknown';
export function claimState(task: Task): ClaimState {
  if (!task.run) return 'free';
  if (task.staleness === 'stale') return 'stale';
  if (task.staleness === 'unknown') return 'unknown';
  return 'running';
}

export function claimNote(task: Task): string {
  switch (claimState(task)) {
    case 'running':
      return `Claimed by session ${task.run?.session ?? '?'} — it moves to In Review when the agent finishes`;
    case 'unknown':
      return `Claimed on ${task.run?.host ?? 'another host'} — the board can't check whether that run is still alive`;
    case 'stale':
      return `The claiming session (pid ${task.run?.pid ?? '?'} on ${task.run?.host ?? '?'}) is gone — release the task so it can be picked up again`;
    default:
      return '';
  }
}

export type ReorderDir = 'top' | 'up' | 'down' | 'bottom';
export type OrderStep = { id: string; target: { before?: string; afterPos?: string; top?: boolean; bottom?: boolean } };

/** Can this card be reordered at all? (web: not while running, not in final lanes) */
export function canReorder(task: Task): boolean {
  return !task.run && !['done', 'cancelled', 'archived'].includes(task.status);
}

/** The lane as drawn, split into blocks: a single card or one same-lane dependency chain. */
export function laneBlocks(laneTaskList: Task[], chains: boolean): Task[][] {
  const blocks: Task[][] = [];
  for (const item of laneLayout(laneTaskList, chains) as LaneItem[]) {
    if (item.chain === 'single' || item.chain === 'first') blocks.push([item.task]);
    else blocks.at(-1)?.push(item.task);
  }
  return blocks;
}

/**
 * The `order` writes for one reorder step, in stored-order terms. A chain
 * moves as a block (every member, keeping its order), so the drawn lane
 * never splits a chain — the web snaps drops to block starts for the same
 * reason. Returns [] when the card is already at that edge.
 */
export function reorderPlan(laneTaskList: Task[], chains: boolean, taskId: string, dir: ReorderDir): OrderStep[] {
  const blocks = laneBlocks(laneTaskList, chains);
  const at = blocks.findIndex((block) => block.some((task) => task.id === taskId));
  if (at === -1) return [];
  const block = blocks[at]!;
  const last = blocks.length - 1;
  if ((dir === 'top' || dir === 'up') && at === 0) return [];
  if ((dir === 'bottom' || dir === 'down') && at === last) return [];
  const first: OrderStep['target'] =
    dir === 'top'
      ? { top: true }
      : dir === 'bottom'
        ? { bottom: true }
        : dir === 'up'
          ? { before: blocks[at - 1]![0]!.id }
          : { afterPos: blocks[at + 1]!.at(-1)!.id };
  return block.map((task, index) => ({ id: task.id, target: index === 0 ? first : { afterPos: block[index - 1]!.id } }));
}

/** Where a card sits among the lane's blocks (for disabling edge actions). */
export function blockPosition(laneTaskList: Task[], chains: boolean, taskId: string): { index: number; count: number } {
  const blocks = laneBlocks(laneTaskList, chains);
  return { index: blocks.findIndex((block) => block.some((task) => task.id === taskId)), count: blocks.length };
}

/** Tasks that run after `id` (its dependents). */
export function dependentsOf(tasks: readonly Task[], id: string): Task[] {
  return tasks.filter((task) => (task.deps ?? []).includes(id));
}
