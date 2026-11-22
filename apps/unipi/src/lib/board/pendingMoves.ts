/**
 * The Workbench's undo window: Mark done and Rework wait 5 s before they are
 * saved. During the window the ticket is hidden from the queue and the counts
 * (`isPending`); Undo cancels the save and puts the typed note back. Closing
 * the Workbench, leaving the page or the app going hidden saves what is still
 * waiting at once (`flushMoves`). A move the daemon refuses comes back to the
 * queue with an error toast. Frontend only: the daemon sees one ordinary move.
 */
import { createSignal } from 'solid-js';
import { api, describe, laneLabel, type Task } from './api';
import { boardStore } from './store';
import { boardUi } from './ui';

export const UNDO_MS = 5000;

type Pending = { key: string; hostId: string; slug: string; project: string; task: Task; to: string; note: string; timer: ReturnType<typeof setTimeout>; toast: number };
export type UndoneMove = { project: string; task: string; note: string };

const [waiting, setWaiting] = createSignal<Pending[]>([]);
const undoListeners = new Set<(move: UndoneMove) => void>();
const settledListeners = new Set<() => void>();
const keyOf = (slug: string, task: string) => `${slug}/${task}`;

/** The note a ticket keeps for the app session (`slug/ID`). */
export const noteKey = keyOf;

/** Is this ticket waiting in the undo window? (reactive) */
export const isPending = (project: string, task: string): boolean => waiting().some((p) => p.key === keyOf(project, task));
export const pendingCount = (): number => waiting().length;

/** Runs when an Undo put a ticket back (the Workbench re-opens it). Returns the unsubscribe. */
export function onMoveUndone(listener: (move: UndoneMove) => void): () => void {
  undoListeners.add(listener);
  return () => undoListeners.delete(listener);
}
/** Runs once a waiting move was saved or refused (the Workbench re-reads its queue). */
export function onMovesSettled(listener: () => void): () => void {
  settledListeners.add(listener);
  return () => settledListeners.delete(listener);
}

const drop = (key: string) => setWaiting((old) => old.filter((p) => p.key !== key));

/** `early`: saved ahead of the window (the Workbench closed, the app went hidden): Undo is gone, so the toast says it was saved. */
async function save(p: Pending, early = false): Promise<void> {
  clearTimeout(p.timer);
  boardUi().dismissToast(p.toast);
  try {
    const moved = await api.move(p.hostId, p.slug, p.task.id, p.to, p.note.trim() || undefined);
    const at = boardStore().target();
    if (at?.hostId === p.hostId && at.slug === p.slug) {
      boardStore().applyTask(moved);
      void boardStore().reload();
    }
    boardUi().setNote(p.key, '');
    if (early) boardUi().toast(`${p.task.id} ${p.to === 'done' ? 'marked done' : `sent back to ${laneLabel(p.to)}`} — saved`, 'success');
  } catch (error) {
    boardUi().toast(`${p.task.id} was not moved: ${describe(error)}`, 'error');
    // Back in the queue, with the note it had.
    boardUi().setNote(p.key, p.note);
    drop(p.key);
    undoListeners.forEach((fn) => fn({ project: p.project, task: p.task.id, note: p.note }));
  } finally {
    drop(p.key);
    settledListeners.forEach((fn) => fn());
  }
}

/** Starts the window for `task` → `to`; the toast carries Undo. */
export function scheduleMove(input: { hostId: string; slug: string; task: Task; to: string; note: string }): void {
  const key = keyOf(input.slug, input.task.id);
  if (waiting().some((p) => p.key === key)) return;
  const verb = input.to === 'done' ? 'marked done' : `sent back to ${laneLabel(input.to)}`;
  const toast = boardUi().toast(`${input.task.id} ${verb}`, 'success', { label: 'Undo', run: () => undoMove(key) });
  const pending: Pending = {
    key, hostId: input.hostId, slug: input.slug, project: input.slug, task: input.task, to: input.to, note: input.note, toast,
    timer: setTimeout(() => void save(pending), UNDO_MS),
  };
  boardUi().setNote(key, '');
  setWaiting((old) => [...old, pending]);
}

/** Cancels the waiting save: the ticket and its note come back. */
export function undoMove(key: string): void {
  const p = waiting().find((item) => item.key === key);
  if (!p) return;
  clearTimeout(p.timer);
  boardUi().dismissToast(p.toast);
  drop(key);
  boardUi().setNote(key, p.note);
  undoListeners.forEach((fn) => fn({ project: p.project, task: p.task.id, note: p.note }));
}

/** Saves every waiting move now (Workbench closed, page left, app hidden). */
export async function flushMoves(): Promise<void> {
  await Promise.all(waiting().map((p) => save(p, true)));
}

if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') void flushMoves();
  });
}

/** Test hook: forget every waiting move without saving it. */
export function resetPendingMoves(): void {
  waiting().forEach((p) => clearTimeout(p.timer));
  setWaiting([]);
  undoListeners.clear();
  settledListeners.clear();
}
