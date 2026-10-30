/**
 * Board keyboard shortcuts on the desktop shell (UNI-117 kanboard
 * migration; keymap in lib/board/keys.ts) and the "Keyboard shortcuts"
 * sheet (`?`). Mounted by the Board screen; listens only while the desktop
 * shell is on, so the phone app is unaffected.
 */
import { createEffect, For, onCleanup, onMount } from 'solid-js';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { boardActions } from '../../lib/board/actions';
import { laneLayout, laneTasks } from '../../lib/board/view';
import { BOARD_SHORTCUTS, boardKey, isTyping, overlayOpen, stepSelection } from '../../lib/board/keys';
import { desktopShell } from '../../lib/layout';
import { modKey } from '../../lib/shell';
import { BoardSheet } from './BoardSheet';
import './dashboard.css';

export function BoardKeys(props: { lanes: () => { id: string }[]; project: () => boolean }) {
  const store = boardStore();
  const ui = boardUi();
  /** Visible cards in reading order: lane by lane (board) / group by group (list). */
  const order = () =>
    props.lanes().flatMap((lane) => laneLayout(laneTasks(store.state.tasks, lane.id, ui.query(), ui.filters), ui.display.chains).map((item) => item.task.id));

  // Mark + reveal the selected card / row.
  createEffect(() => {
    const id = ui.selected();
    if (!id || !desktopShell()) return;
    const nodes = [...document.querySelectorAll<HTMLElement>(`.board-screen [data-id="${id.replace(/"/g, '')}"]`)];
    nodes.forEach((node) => node.classList.add('kb-selected'));
    nodes[0]?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    onCleanup(() => nodes.forEach((node) => node.classList.remove('kb-selected')));
  });
  // The opened task becomes the selection (so J/K continue from it).
  createEffect(() => {
    const opened = ui.openedTask();
    if (opened) ui.setSelected(opened);
  });

  onMount(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!desktopShell() || event.defaultPrevented) return;
      const action = boardKey(event, {
        typing: isTyping(event.target),
        overlay: overlayOpen() || ui.sheet() !== null || ui.shortcutsOpen(),
        taskOpen: !!ui.openedTask(),
        project: props.project(),
        filtered: ui.query().trim().length > 0,
      });
      if (!action) return;
      event.preventDefault();
      switch (action.kind) {
        case 'new':
          ui.openNewTask({ lane: ui.view() === 'board' && ui.activeLane() === 'todo' ? 'todo' : 'backlog' });
          break;
        case 'search':
          ui.setSheet('search');
          break;
        case 'select': {
          const next = stepSelection(order(), ui.selected(), action.delta);
          if (next) ui.setSelected(next);
          break;
        }
        case 'open': {
          const task = store.taskById(ui.selected());
          if (task) boardActions().openTask(task);
          break;
        }
        case 'view':
          ui.setView(action.view);
          break;
        case 'scope':
          ui.setView('list', ui.scope() === action.scope ? 'all' : action.scope);
          break;
        case 'help':
          ui.setShortcutsOpen(true);
          break;
        case 'clear':
          ui.setQuery('');
          break;
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  return null;
}

export function BoardShortcutsSheet() {
  const ui = boardUi();
  return (
    <BoardSheet open={ui.shortcutsOpen()} onClose={() => ui.setShortcutsOpen(false)} label="Keyboard shortcuts" testId="shortcuts-sheet">
      <div class="kb-shortcuts">
        <For each={BOARD_SHORTCUTS}>
          {([label, keys]) => (
            <>
              <span>{label}</span>
              <span class="kb-shortcut-keys">
                <For each={keys}>{(key) => <kbd class="kbd">{key === 'Ctrl' ? modKey() : key}</kbd>}</For>
              </span>
            </>
          )}
        </For>
      </div>
    </BoardSheet>
  );
}
