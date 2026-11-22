/**
 * Task action menus (UNI-131), all Sessions-style anchored popovers:
 *  - CardMenu: the card long-press quick menu — Open, the allowed moves
 *    (with "note" marks), one-tap priority, reorder, duplicate /
 *    recreate, archive, cancel;
 *  - MoveMenu / PriorityMenu: the detail's status and priority chips;
 *  - TaskMoreMenu: the detail header ⋯ (web TaskPanel "Task actions").
 * Running cards list no moves (the agent moves them on); a stale claim
 * offers "Release to Todo / Backlog" (a note is required).
 */
import { createEffect, For, onCleanup, Show, type JSX } from 'solid-js';
import { ArrowDown, ArrowDownToLine, ArrowUp, ArrowUpToLine, Archive, Copy, CopyPlus, FileText, Maximize2, MessageSquarePlus, RotateCcw, XCircle } from 'lucide-solid';
import { canMove, displayTitle, laneLabel, needsComment, PRIORITIES, PRIORITY_LABEL, type Task } from '../../lib/board/api';
import { requestChatPick } from '../../lib/board/portal';
import { can } from '../../lib/platform';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { blockPosition, canReorder, claimNote, claimState, statusMoves, type ReorderDir } from '../../lib/board/report';
import { laneTasks, EMPTY_FILTERS } from '../../lib/board/view';
import { copyToClipboard, duplicateTask, editTask, moveTask, recreateTask, reorderTask } from '../../lib/board/taskActions';
import { AnchoredMenu, MenuLabel, MenuRow, MenuSeparator } from './AnchoredMenu';
import { PriorityGlyph, StatusGlyph } from './Glyphs';

export const moveLabel = (from: string, to: string) =>
  from === 'in_progress' ? `Release to ${laneLabel(to)}` : from === 'in_review' && to === 'todo' ? 'Back to Todo (rework)' : `Move to ${laneLabel(to)}`;

/** The move rows for a task (shared by the card menu and the status chip menu). */
export function MoveRows(props: { task: Task; close: () => void; includeCancel?: boolean }) {
  const store = boardStore();
  const claim = () => claimState(props.task);
  const moves = () => statusMoves(store.state.rules, props.task).filter((to) => props.includeCancel || to !== 'cancelled');
  return (
    <Show
      when={claim() === 'free' || claim() === 'stale'}
      fallback={<p class="board-menu-note">{claimNote(props.task)}</p>}
    >
      <Show when={claim() === 'stale'}>
        <p class="board-menu-note warn">Stale run — the agent session is gone. Releasing needs a note.</p>
      </Show>
      <For each={moves()} fallback={<p class="board-menu-note">No moves from {laneLabel(props.task.status)}.</p>}>
        {(to) => (
          <MenuRow
            icon={<StatusGlyph status={to} size={16} />}
            label={moveLabel(props.task.status, to)}
            tone={to === 'cancelled' ? 'danger' : undefined}
            hint={needsComment(store.state.rules, props.task.status, to) ? 'note' : undefined}
            onSelect={() => {
              const task = props.task;
              props.close();
              void moveTask(task, to);
            }}
          />
        )}
      </For>
    </Show>
  );
}

function IconChoice(props: { label: string; checked?: boolean; disabled?: boolean; onSelect: () => void; children: JSX.Element }) {
  return (
    <button
      type="button"
      role={props.checked === undefined ? 'menuitem' : 'menuitemradio'}
      aria-checked={props.checked}
      aria-label={props.label}
      title={props.label}
      class="menu-choice"
      disabled={props.disabled}
      onClick={() => props.onSelect()}
    >
      {props.children}
    </button>
  );
}

/** One-tap priority (5 glyph buttons in a row). */
export function PriorityRow(props: { task: Task; close: () => void }) {
  return (
    <div class="menu-choice-row" role="group" aria-label="Priority">
      <For each={[...PRIORITIES]}>
        {(priority) => (
          <IconChoice
            label={`Priority: ${PRIORITY_LABEL[priority]}`}
            checked={(props.task.priority || 'none') === priority}
            onSelect={() => {
              const task = props.task;
              props.close();
              if (priority !== task.priority) void editTask(task, { priority });
            }}
          >
            <PriorityGlyph priority={priority} size={17} />
          </IconChoice>
        )}
      </For>
    </div>
  );
}

/** Top / up / down / bottom of the lane (the phone's drag & drop). */
export function ReorderRow(props: { task: Task; close: () => void }) {
  const store = boardStore();
  const ui = boardUi();
  const pos = () => blockPosition(laneTasks(store.state.tasks, props.task.status, '', EMPTY_FILTERS), ui.display.chains, props.task.id);
  const first = () => pos().index <= 0;
  const last = () => pos().index >= pos().count - 1;
  const go = (dir: ReorderDir) => () => {
    const task = props.task;
    props.close();
    void reorderTask(task, dir);
  };
  return (
    <div class="menu-choice-row" role="group" aria-label={`Order in ${laneLabel(props.task.status)}`}>
      <IconChoice label="Move to top" disabled={first()} onSelect={go('top')}>
        <ArrowUpToLine size={18} />
      </IconChoice>
      <IconChoice label="Move up" disabled={first()} onSelect={go('up')}>
        <ArrowUp size={18} />
      </IconChoice>
      <IconChoice label="Move down" disabled={last()} onSelect={go('down')}>
        <ArrowDown size={18} />
      </IconChoice>
      <IconChoice label="Move to bottom" disabled={last()} onSelect={go('bottom')}>
        <ArrowDownToLine size={18} />
      </IconChoice>
    </div>
  );
}

/** Board → chat portal (UNI-181/185): asks which chat on this PC, then drops the task id into its draft. */
function addToChat(task: Task) {
  const hostId = boardStore().target()?.hostId;
  if (hostId) requestChatPick({ hostId, mention: `${task.id} `, chip: { kind: 'task', label: task.id, markdown: `Board task ${task.id}${task.title ? `: ${task.title}` : ''}` } });
}

function TailRows(props: { task: Task; close: () => void; detail?: boolean }) {
  const store = boardStore();
  /** The menu (and the task getter behind it) unmounts on close: act on a snapshot of the task. */
  const run = (fn: (task: Task) => void) => () => {
    const task = props.task;
    props.close();
    fn(task);
  };
  const archivable = () => props.task.status !== 'archived' && canMove(store.state.rules, props.task, 'archived') && !props.task.run;
  const cancellable = () => props.task.status !== 'cancelled' && canMove(store.state.rules, props.task, 'cancelled') && !props.task.run;
  return (
    <>
      <MenuRow icon={<CopyPlus size={18} />} label="Duplicate" onSelect={run((task) => void duplicateTask(task))} />
      <Show when={props.task.status === 'cancelled'}>
        <MenuRow icon={<RotateCcw size={18} />} label="Recreate task…" onSelect={run((task) => recreateTask(task))} />
      </Show>
      <Show when={props.detail && boardStore().target()?.hostId && can('chat')}>
        <MenuRow icon={<MessageSquarePlus size={18} />} label="Add to chat" onSelect={run((task) => addToChat(task))} />
      </Show>
      <Show when={props.detail}>
        <MenuRow icon={<Copy size={18} />} label="Copy id" onSelect={run((task) => void copyToClipboard(task.id, task.id))} />
        <MenuRow
          icon={<FileText size={18} />}
          label="Copy file path"
          disabled={!props.task.path}
          onSelect={run((task) => void copyToClipboard(task.path ?? '', 'the file path'))}
        />
      </Show>
      <Show when={archivable() || cancellable()}>
        <MenuSeparator />
      </Show>
      <Show when={archivable()}>
        <MenuRow icon={<Archive size={18} />} label="Archive" onSelect={run((task) => void moveTask(task, 'archived'))} />
      </Show>
      <Show when={cancellable()}>
        <MenuRow icon={<XCircle size={18} />} label="Cancel task" tone="danger" onSelect={run((task) => void moveTask(task, 'cancelled'))} />
      </Show>
    </>
  );
}

/** Card long-press quick menu, anchored at the card (ui.cardMenu). */
export function CardMenu() {
  const ui = boardUi();
  const store = boardStore();
  const task = () => store.taskById(ui.cardMenu()?.taskId);
  const close = () => ui.setCardMenu(null);
  // Mark the pressed card while its menu is open (the menu may cover part of it).
  createEffect(() => {
    const id = ui.cardMenu()?.taskId;
    if (!id) return;
    const nodes = [...document.querySelectorAll<HTMLElement>(`[data-id="${id.replace(/"/g, "")}"]`)];
    nodes.forEach((node) => node.classList.add('menu-target'));
    onCleanup(() => nodes.forEach((node) => node.classList.remove('menu-target')));
  });
  return (
    <Show when={ui.cardMenu() ? task() : undefined} keyed>
      {(current) => (
        <AnchoredMenu anchor={ui.cardMenu()!.anchor} width={292} label={`${current.id} actions`} onDismiss={close}>
          <div class="board-menu-head">
            <StatusGlyph status={current.status} size={14} />
            <span class="font-mono">{current.id}</span>
            <span class="truncate" classList={{ derived: !current.title }}>
              {displayTitle(current)}
            </span>
          </div>
          <MenuRow
            icon={<Maximize2 size={18} />}
            label="Open"
            onSelect={() => {
              close();
              ui.openTask(current.id);
            }}
          />
          <MenuSeparator />
          <MoveRows task={current} close={close} />
          <MenuSeparator />
          <PriorityRow task={current} close={close} />
          <Show when={canReorder(current)}>
            <ReorderRow task={current} close={close} />
          </Show>
          <MenuSeparator />
          <TailRows task={current} close={close} />
        </AnchoredMenu>
      )}
    </Show>
  );
}

/** Detail header ⋯. */
export function TaskMoreMenu(props: { task: Task; anchor: DOMRect; onDismiss: () => void }) {
  return (
    <AnchoredMenu anchor={props.anchor} alignEnd width={272} label="Task actions" onDismiss={props.onDismiss}>
      <Show when={canReorder(props.task)}>
        <MenuLabel>Order in {laneLabel(props.task.status)}</MenuLabel>
        <ReorderRow task={props.task} close={props.onDismiss} />
        <MenuSeparator />
      </Show>
      <TailRows task={props.task} close={props.onDismiss} detail />
    </AnchoredMenu>
  );
}

/** Detail status chip → "Move to". */
export function MoveMenu(props: { task: Task; anchor: DOMRect; onDismiss: () => void }) {
  return (
    <AnchoredMenu anchor={props.anchor} width={272} label="Move to" onDismiss={props.onDismiss}>
      <MenuLabel>Move {props.task.id} to</MenuLabel>
      <MoveRows task={props.task} close={props.onDismiss} includeCancel />
    </AnchoredMenu>
  );
}

/** Detail priority chip. */
export function PriorityMenu(props: { task: Task; anchor: DOMRect; onDismiss: () => void }) {
  return (
    <AnchoredMenu anchor={props.anchor} width={232} label="Priority" onDismiss={props.onDismiss}>
      <For each={[...PRIORITIES].reverse()}>
        {(priority) => (
          <MenuRow
            icon={<PriorityGlyph priority={priority} size={16} />}
            label={PRIORITY_LABEL[priority]!}
            hint={(props.task.priority || 'none') === priority ? '✓' : undefined}
            onSelect={() => {
              const task = props.task;
              props.onDismiss();
              if (priority !== task.priority) void editTask(task, { priority });
            }}
          />
        )}
      </For>
    </AnchoredMenu>
  );
}
