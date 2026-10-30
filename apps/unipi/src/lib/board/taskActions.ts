/**
 * Every task and project write of the Board (UNI-131), ported from the
 * kanboard web UI (TaskPanel.tsx moveTo/run, Board.tsx commit/doneAll/
 * archiveLane, Dialogs.tsx CommentDialog, schedule.ts, App.tsx
 * setArchived). The UI components only call these.
 *
 * Rules kept from the web:
 *  - moves only to lanes the rules allow; a move that needs a note opens
 *    the comment sheet first (or after a 409 `needsComment` refusal);
 *  - phone addition: a move picked from a menu into a final lane (Done,
 *    Cancelled, Archived) asks first — a mis-tap in a long menu is easy
 *    on a phone. Explicit buttons ("Mark done") and the comment sheet are
 *    their own confirmation (`confirmed`);
 *  - final-lane moves ask first (the CLI contract has no one-shot undo
 *    token, so a hard-to-reverse move is confirmed before it happens);
 *  - a task landing in Todo while a dependency is still in Backlog
 *    offers (never forces) to schedule those dependencies;
 *  - every write upserts the server's answer (`applyTask`) and then
 *    reloads the project, so the UI is consistent before live updates
 *    (UNI-132) exist.
 */
import { api, ApiError, describe, displayTitle, laneLabel, needsComment, type Attachment, type EditTaskPatch, type Task } from './api';
import { boardStore } from './store';
import { boardUi } from './ui';
import { uploadAll } from './attach';
import { reorderPlan, type ReorderDir } from './report';
import { laneTasks, EMPTY_FILTERS } from './view';
import { copyText } from './share';
import type { DropPlan } from './dnd';

export type BoardCtx = { hostId: string; slug: string };
type HapticFn = (kind?: 'tap' | 'tick') => void;

let haptic: HapticFn = () => {};
/** The board screen registers the haptic tick (it owns the settings). */
export function registerActionHaptic(fn: HapticFn | undefined): void {
  haptic = fn ?? (() => {});
}

/** A haptic tick through the registered handler (long-press menus). */
export function buzz(kind: 'tap' | 'tick' = 'tick'): void {
  haptic(kind);
}

/** The open (PC, project), captured at action time. */
export function boardCtx(): BoardCtx | undefined {
  const at = boardStore().target();
  return at?.slug ? { hostId: at.hostId, slug: at.slug } : undefined;
}
const onBoard = (ctx: BoardCtx) => {
  const at = boardStore().target();
  return at?.hostId === ctx.hostId && at.slug === ctx.slug;
};
const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`;
const toastError = (error: unknown) => boardUi().toast(describe(error), 'error');
const snapshot = (task: Task): Task => JSON.parse(JSON.stringify(task)) as Task;

async function settle(ctx: BoardCtx): Promise<void> {
  if (onBoard(ctx)) await boardStore().reload();
}

// ─── moves ───────────────────────────────────────────────────────────────────

const FINAL_DETAIL: Record<string, string> = {
  done: 'Done is final for a task — afterwards it can only be archived.',
  cancelled: 'Cancelled is final for a task — afterwards it can only be archived.',
  archived: 'Archived tasks leave the board for good.',
};

/**
 * Moves a task. Without a `comment`, a move that the rules say needs one
 * opens the comment sheet instead (returns false); an unconfirmed move into
 * a final lane opens the confirm sheet (returns false). Returns true once moved.
 */
export async function moveTask(
  task: Task,
  to: string,
  comment?: string,
  opts: { onError?: (message: string) => void; confirmed?: boolean; after?: (moved: Task) => void | Promise<void> } = {},
): Promise<boolean> {
  const store = boardStore();
  const ui = boardUi();
  const ctx = boardCtx();
  if (!ctx || to === task.status) return false;
  const original = snapshot(task);
  const hint = needsComment(store.state.rules, original.status, to);
  if (hint && !comment?.trim()) {
    ui.setComment({ task: original, to, hint, after: opts.after });
    return false;
  }
  if (!opts.confirmed && !comment?.trim() && (store.state.rules.final ?? []).includes(to)) {
    ui.setConfirm({
      title: `Move ${original.id} to ${laneLabel(to)}?`,
      detail: FINAL_DETAIL[to] ?? `${laneLabel(to)} is final for a task.`,
      confirm: to === 'archived' ? 'Archive' : to === 'cancelled' ? 'Cancel task' : `Move to ${laneLabel(to)}`,
      dismiss: to === 'cancelled' ? 'Keep it' : undefined,
      danger: to !== 'done',
      run: async () => {
        await moveTask(original, to, undefined, { ...opts, confirmed: true });
      },
    });
    return false;
  }
  const rollback = store.optimistic(original.id, { status: to });
  try {
    const moved = await api.move(ctx.hostId, ctx.slug, original.id, to, comment?.trim() || undefined);
    if (onBoard(ctx)) store.applyTask(moved);
    haptic('tap');
    if (opts.after) {
      try {
        await opts.after(moved);
      } catch (error) {
        toastError(error);
      }
    }
    ui.toast(`Moved ${original.id} to ${laneLabel(to)}`, 'success');
    await settle(ctx);
    if (to === 'todo' && onBoard(ctx)) offerToSchedule(ctx, original.id);
    return true;
  } catch (error) {
    if (onBoard(ctx)) rollback();
    if (error instanceof ApiError && error.needsComment) {
      // Older daemon without /api/rules: the refusal tells us a note is needed.
      ui.setComment({ task: original, to, hint: 'note', after: opts.after });
      return false;
    }
    if (opts.onError) opts.onError(describe(error));
    else toastError(error);
    await settle(ctx);
    return false;
  }
}

/** web schedule.ts: a Todo task whose deps are parked in Backlog is locked — offer to move them too. */
export function offerToSchedule(ctx: BoardCtx, taskId: string): void {
  const store = boardStore();
  const task = store.taskById(taskId);
  if (!task) return;
  const parked = (task.deps ?? []).filter((dep) => store.taskById(dep)?.status === 'backlog');
  if (parked.length === 0) return;
  const list = parked.join(', ');
  boardUi().toast(`${task.id} is locked: ${list} ${parked.length === 1 ? 'is' : 'are'} still in Backlog`, 'warning', {
    label: `Move ${parked.length === 1 ? list : `${parked.length} tasks`} to Todo`,
    run: async () => void (await scheduleDeps(ctx, parked, task.id)),
  });
}

/** Moves Backlog dependencies to Todo (the lock note's button + the schedule offer). */
export async function scheduleDeps(ctx: BoardCtx, ids: string[], unlocks?: string): Promise<boolean> {
  try {
    for (const dep of ids) {
      const moved = await api.move(ctx.hostId, ctx.slug, dep, 'todo');
      if (onBoard(ctx)) boardStore().applyTask(moved);
    }
    haptic('tap');
    boardUi().toast(
      unlocks
        ? `Moved ${ids.join(', ')} to Todo — ${unlocks} unlocks when ${ids.length === 1 ? 'it reaches' : 'they reach'} review`
        : `Scheduled ${ids.join(', ')}`,
      'success',
    );
    return true;
  } catch (error) {
    toastError(error);
    return false;
  } finally {
    await settle(ctx);
  }
}

// ─── edits ───────────────────────────────────────────────────────────────────

let editChain: Promise<unknown> = Promise.resolve();
/**
 * Edits title / body / priority / labels (the daemon lets the user actor
 * edit every field; it refuses a task with neither title nor body).
 * Edits run one at a time, so quick label toggles never race.
 */
export function editTask(task: Task, patch: EditTaskPatch | ((latest: Task) => EditTaskPatch | null), success?: string): Promise<Task | undefined> {
  const run = async (): Promise<Task | undefined> => {
    const store = boardStore();
    const ctx = boardCtx();
    if (!ctx) return undefined;
    const latest = store.taskById(task.id) ?? task;
    const body = typeof patch === 'function' ? patch(latest) : patch;
    if (!body) return latest;
    try {
      const edited = await api.edit(ctx.hostId, ctx.slug, task.id, body);
      if (onBoard(ctx)) store.applyTask(edited);
      if (success) boardUi().toast(success, 'success');
      void settle(ctx);
      return edited;
    } catch (error) {
      toastError(error);
      void settle(ctx);
      return undefined;
    }
  };
  const next = editChain.then(run, run);
  editChain = next.catch(() => undefined);
  return next;
}

export function toggleLabel(task: Task, label: string): Promise<Task | undefined> {
  return editTask(task, (latest) => {
    const current = latest.labels ?? [];
    return { labels: current.includes(label) ? current.filter((item) => item !== label) : [...current, label] };
  });
}

export function addLabel(task: Task, raw: string): Promise<Task | undefined> {
  const label = raw.trim().replace(/,/g, '');
  return editTask(task, (latest) => (!label || (latest.labels ?? []).includes(label) ? null : { labels: [...(latest.labels ?? []), label] }));
}

export function removeLabel(task: Task, label: string): Promise<Task | undefined> {
  return editTask(task, (latest) => ({ labels: (latest.labels ?? []).filter((item) => item !== label) }));
}

export async function addNote(task: Task, text: string): Promise<boolean> {
  const ctx = boardCtx();
  const clean = text.trim();
  if (!ctx || !clean) return false;
  try {
    const noted = await api.note(ctx.hostId, ctx.slug, task.id, clean);
    if (onBoard(ctx)) boardStore().applyTask(noted);
    haptic('tap');
    void settle(ctx);
    return true;
  } catch (error) {
    toastError(error);
    return false;
  }
}

/** Link/unlink a dependency ("runs after"). The daemon refuses cycles; the picker never offers them. */
export async function setDependency(task: Task, dep: string, linked: boolean): Promise<boolean> {
  const ctx = boardCtx();
  if (!ctx) return false;
  try {
    const result = linked
      ? await api.link(ctx.hostId, ctx.slug, task.id, dep)
      : await api.unlink(ctx.hostId, ctx.slug, task.id, dep);
    if (onBoard(ctx)) boardStore().applyTask(result);
    haptic('tick');
    boardUi().toast(linked ? `${task.id} now runs after ${dep}` : `${task.id} no longer waits for ${dep}`, 'success');
    void settle(ctx);
    return true;
  } catch (error) {
    toastError(error);
    void settle(ctx);
    return false;
  }
}

// ─── drag & drop (desktop / web, lib/board/dnd.ts) ────────────────────────────

/**
 * Applies a card drop: a reorder in place, or a move to another lane
 * (through moveTask: note sheet, final-lane confirm, undo toast, schedule
 * offer) followed by the drop position. A drop into a final lane is its
 * own confirmation (the user dragged it there on purpose).
 */
export async function dropTask(task: Task, plan: DropPlan): Promise<boolean> {
  const store = boardStore();
  const ctx = boardCtx();
  if (!ctx || plan.kind === 'none') return false;
  if (plan.kind === 'refused') {
    boardUi().toast(plan.message, 'error');
    return false;
  }
  if (plan.kind === 'reorder') {
    try {
      const placed = await api.order(ctx.hostId, ctx.slug, task.id, plan.order);
      if (onBoard(ctx)) store.applyTask(placed);
      haptic('tick');
      boardUi().toast(`Reordered ${task.id}`, 'success');
      return true;
    } catch (error) {
      toastError(error);
      return false;
    } finally {
      await settle(ctx);
    }
  }
  const order = plan.order;
  return moveTask(task, plan.to, undefined, {
    confirmed: true,
    after: order ? async () => {
      const placed = await api.order(ctx.hostId, ctx.slug, task.id, order);
      if (onBoard(ctx)) store.applyTask(placed);
    } : undefined,
  });
}

// ─── reorder, duplicate, recreate ────────────────────────────────────────────

/** Move a card (its whole chain block) to the top / up / down / bottom of its lane. */
export async function reorderTask(task: Task, dir: ReorderDir): Promise<boolean> {
  const store = boardStore();
  const ctx = boardCtx();
  if (!ctx) return false;
  const lane = laneTasks(store.state.tasks, task.status, '', EMPTY_FILTERS);
  const steps = reorderPlan(lane, boardUi().display.chains, task.id, dir);
  if (steps.length === 0) return false;
  try {
    for (const step of steps) {
      const placed = await api.order(ctx.hostId, ctx.slug, step.id, step.target);
      if (onBoard(ctx)) store.applyTask(placed);
    }
    haptic('tick');
    const where = { top: 'to the top', up: 'up', down: 'down', bottom: 'to the bottom' }[dir];
    boardUi().toast(`Moved ${task.id} ${where} of ${laneLabel(task.status)}`, 'success');
    return true;
  } catch (error) {
    toastError(error);
    return false;
  } finally {
    await settle(ctx);
  }
}

export async function duplicateTask(task: Task): Promise<Task | undefined> {
  const ctx = boardCtx();
  if (!ctx) return undefined;
  try {
    const copy = await api.duplicate(ctx.hostId, ctx.slug, task.id);
    if (onBoard(ctx)) boardStore().applyTask(copy);
    haptic('tap');
    boardUi().toast(`Duplicated ${task.id} as ${copy.id}`, 'success', {
      label: 'Open',
      run: () => void boardUi().openTask(copy.id),
    });
    void settle(ctx);
    return copy;
  } catch (error) {
    toastError(error);
    return undefined;
  }
}

/** UNI-57 Recreate (cancelled tasks): the New task sheet prefilled with the creation fields. */
export function recreateTask(task: Task): void {
  boardUi().openNewTask({ lane: 'todo', recreateFrom: snapshot(task) });
}

export async function copyToClipboard(text: string, what: string): Promise<void> {
  const ok = await copyText(text);
  boardUi().toast(ok ? `Copied ${what}` : `Couldn't copy ${what}`, ok ? 'success' : 'error');
}

// ─── attachments ─────────────────────────────────────────────────────────────

/** Uploads files to a task (sequentially, 25 MB each); errors toast, the rest still upload. */
export async function uploadToTask(task: Task, files: File[]): Promise<Attachment[]> {
  const ctx = boardCtx();
  if (!ctx || files.length === 0) return [];
  const done = await uploadAll(ctx.hostId, ctx.slug, task.id, files, (message) => boardUi().toast(message, 'error'));
  if (done.length > 0) void settle(ctx);
  return done;
}

// ─── lane + project actions ──────────────────────────────────────────────────

/** In Review → Done for every card (web "Done all"). */
export async function doneLane(): Promise<void> {
  const ctx = boardCtx();
  if (!ctx) return;
  try {
    const result = await api.doneLane(ctx.hostId, ctx.slug);
    haptic('tap');
    boardUi().toast(`Moved ${result.moved.length} to Done`, 'success');
  } catch (error) {
    toastError(error);
  } finally {
    await settle(ctx);
  }
}

/** Archive every card of In Review / Done (web "Archive all"). */
export async function archiveLane(status: 'done' | 'in_review'): Promise<void> {
  const ctx = boardCtx();
  if (!ctx) return;
  try {
    const result = await api.archiveLane(ctx.hostId, ctx.slug, status);
    haptic('tap');
    const skipped = result.skipped?.length ? ` · ${result.skipped.length} skipped` : '';
    boardUi().toast(`Archived ${plural(result.archived.length, 'task')}${skipped}`, 'success');
  } catch (error) {
    toastError(error);
  } finally {
    await settle(ctx);
  }
}

/** Archive / unarchive the open project (web sidebar, UNI-51); Undo flips it back. */
export async function setProjectArchived(archived: boolean, ctx = boardCtx()): Promise<void> {
  const store = boardStore();
  if (!ctx) return;
  const name = store.state.projects.find((project) => project.slug === ctx.slug)?.name ?? ctx.slug;
  try {
    await api.updateProject(ctx.hostId, ctx.slug, { archived });
    haptic('tap');
    boardUi().toast(archived ? `Archived ${name}` : `Unarchived ${name}`, 'success', {
      label: 'Undo',
      run: () => setProjectArchived(!archived, ctx),
    });
  } catch (error) {
    toastError(error);
  } finally {
    if (store.target()?.hostId === ctx.hostId) await store.loadProjects();
  }
}

/** For toasts/labels: "UNI-4 · Crash on rotate". */
export function taskLabel(task: Task): string {
  return `${task.id} · ${displayTitle(task)}`;
}
