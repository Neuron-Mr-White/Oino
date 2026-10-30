/**
 * Board action hooks (UNI-130 → UNI-131).
 *
 * The board screen calls these and nothing else for anything beyond
 * browsing/creating: the task detail page, the card long-press quick
 * menu, lane bulk actions and project actions. UNI-130 shipped them as
 * stubs; UNI-131 implements them below on top of lib/board/taskActions.ts
 * (every write), lib/board/summarize.ts (the summary jobs) and the ui
 * signals the board screen renders (detail page, card menu, comment /
 * confirm / settings sheets). Tests can still override single hooks with
 * `setBoardActions`. `highlightTask(id)` jumps to a card and flashes it.
 */
import type { Task } from './api';
import { boardUi } from './ui';
import { boardStore } from './store';
import { summaries } from './summarize';
import { archiveLane, boardCtx, buzz, doneLane, setProjectArchived } from './taskActions';

export type ProjectAction = 'summarize-board' | 'settings' | 'archive-project' | 'unarchive-project';
export type LaneAction = 'done-all' | 'archive-lane' | 'summarize-archive';

export interface BoardActions {
  /** Card tap / search result: open the task detail. */
  openTask(task: Task): void;
  /** Card long-press: quick-action menu anchored at the card's rect. */
  openCardMenu(task: Task, anchor: DOMRect): void;
  /** Header ⋯ menu: Summarize board, Board settings, Archive / Unarchive project. `slug` targets a project other than the open one (project list long-press); defaults to the open project. */
  projectAction(action: ProjectAction, slug?: string): void;
  /** Lane header: In Review → Done all / Archive all; Done → Summarize & archive / Archive all. */
  laneAction(lane: 'in_review' | 'done', action: LaneAction): void;
}

const count = (status: string) => boardStore().state.tasks.filter((task) => task.status === status).length;
const plural = (n: number) => `${n} task${n === 1 ? '' : 's'}`;

/** UNI-131: the real hooks — detail page, long-press menu, lane bulk actions, project actions. */
const defaultActions: BoardActions = {
  openTask: (task) => {
    boardUi().setCardMenu(null);
    boardUi().openTask(task.id);
  },
  openCardMenu: (task, anchor) => {
    buzz('tick');
    boardUi().setCardMenu({ taskId: task.id, anchor });
  },
  projectAction: (action, slug) => {
    const ui = boardUi();
    if (action === 'summarize-board') summaries().show('board');
    else if (action === 'settings') ui.setSettingsOpen(true);
    else {
      const ctx = slug ? { hostId: boardCtx()?.hostId ?? boardStore().target()?.hostId ?? '', slug } : boardCtx();
      if (ctx?.hostId) void setProjectArchived(action === 'archive-project', ctx);
    }
  },
  laneAction: (lane, action) => {
    const ui = boardUi();
    if (action === 'summarize-archive') return summaries().show('done');
    const total = count(lane);
    if (total === 0) return;
    if (action === 'done-all') {
      ui.setConfirm({
        title: `Move ${plural(total)} to Done?`,
        detail: 'Done is final for a task — it can only be archived afterwards.',
        confirm: `Move ${total} to Done`,
        run: () => doneLane(),
      });
    } else {
      ui.setConfirm({
        title: `Archive ${plural(total)} from ${lane === 'done' ? 'Done' : 'In Review'}?`,
        detail: lane === 'done' ? 'No summary is written. Archived tasks leave the board for good.' : 'Archived tasks leave the board for good.',
        confirm: `Archive ${plural(total)}`,
        danger: true,
        run: () => archiveLane(lane),
      });
    }
  },
};

let current: BoardActions = { ...defaultActions };

export function boardActions(): BoardActions {
  return current;
}
/** Overrides some/all hooks (UNI-131, tests). */
export function setBoardActions(partial: Partial<BoardActions>): void {
  current = { ...current, ...partial };
}
export function resetBoardActions(): void {
  current = { ...defaultActions };
}

let highlighter: ((id: string, quiet?: boolean) => void) | undefined;
/** The board screen registers how to jump to + flash a card. */
export function registerHighlighter(fn: ((id: string, quiet?: boolean) => void) | undefined): void {
  highlighter = fn;
}
/** Jump to a task on the open board (its lane page / list row) and flash it; `quiet` skips the "hidden" toasts. */
export function highlightTask(id: string, quiet = false): void {
  highlighter?.(id, quiet);
}
/** A dependency id tapped on a card: open that task's detail (Back flashes its card), or say why not. */
export function openDependency(id: string): void {
  const task = boardStore().taskById(id);
  if (task) current.openTask(task);
  else highlightTask(id);
}
