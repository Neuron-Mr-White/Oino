/**
 * Search (UNI-130) — the web UI's ⌘K CommandPalette plus its toolbar
 * search, merged into one sheet: type to find tasks by id, title, body
 * or label (tap = jump to the card and open it), "Filter the board by …"
 * keeps the text as a board filter, and the palette's actions/projects
 * are listed below.
 */
import { createEffect, createMemo, createSignal, For, Show, type JSX } from 'solid-js';
import { Columns3, Eye, Filter, List, OctagonX, Plus, Search, SlidersHorizontal, X } from 'lucide-solid';
import { displayTitle, laneLabel, type Task } from '../../lib/board/api';
import { fuzzy, hue, searchTasks } from '../../lib/board/view';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { boardActions, highlightTask } from '../../lib/board/actions';
import { BoardSheet } from './BoardSheet';
import { ProjectTile, StatusGlyph } from './Glyphs';

type Command = { id: string; label: string; icon: JSX.Element; hint?: string; run: () => void };

export function SearchSheet(props: { open: boolean; onClose: () => void; onOpenProject: (slug: string) => void }) {
  const store = boardStore();
  const ui = boardUi();
  const [needle, setNeedle] = createSignal('');
  let input: HTMLInputElement | undefined;

  createEffect(() => {
    if (!props.open) return;
    setNeedle(ui.query());
    setTimeout(() => input?.focus({ preventScroll: true }), 60);
  });

  const count = (lane: string) => store.state.tasks.filter((task) => task.status === lane).length;
  const commands = createMemo<Command[]>(() => {
    const list: Command[] = [];
    if (store.target()?.slug) {
      list.push(
        { id: 'new', label: 'Create task', icon: <Plus size={18} />, run: () => ui.openNewTask({ lane: 'backlog' }) },
        { id: 'board', label: 'Go to board', icon: <Columns3 size={18} />, run: () => ui.setView('board') },
        { id: 'list', label: 'Go to list', icon: <List size={18} />, run: () => ui.setView('list') },
        {
          id: 'review',
          label: 'Review queue',
          hint: count('in_review') ? String(count('in_review')) : undefined,
          icon: <Eye size={18} />,
          run: () => ui.setView('list', 'in_review'),
        },
        {
          id: 'blocked',
          label: 'Blocked tasks',
          hint: count('blocked') ? String(count('blocked')) : undefined,
          icon: <OctagonX size={18} />,
          run: () => ui.setView('list', 'blocked'),
        },
        { id: 'filter', label: 'Filter…', icon: <Filter size={18} />, run: () => ui.setSheet('filter') },
        { id: 'display', label: 'Display options…', icon: <SlidersHorizontal size={18} />, run: () => ui.setSheet('display') },
      );
    }
    for (const project of store.state.projects) {
      if (project.slug === store.target()?.slug || project.archived) continue;
      list.push({
        id: `p:${project.slug}`,
        label: `Open ${project.name}`,
        icon: <ProjectTile name={project.name} hue={hue(project.name)} size={22} />,
        run: () => props.onOpenProject(project.slug),
      });
    }
    return list;
  });
  const text = () => needle().trim().toLowerCase();
  const tasks = createMemo(() => searchTasks(store.state.tasks, needle()));
  const shownCommands = createMemo(() => {
    const value = text();
    if (!value) return commands();
    const exact = commands().filter((command) => command.label.toLowerCase().includes(value));
    return exact.length > 0 ? exact : commands().filter((command) => fuzzy(command.label.toLowerCase(), value)).slice(0, 4);
  });

  function close() {
    props.onClose();
  }
  function choose(command: Command) {
    close();
    command.run();
  }
  function jump(task: Task) {
    close();
    // Clear a stale board filter that would hide the result.
    setTimeout(() => {
      highlightTask(task.id);
      boardActions().openTask(task);
    }, 0);
  }

  return (
    <BoardSheet open={props.open} onClose={close} label="Search" testId="search-sheet" class="board-sheet-tall">
      <div class="search-field">
        <Search size={18} class="text-text-dim shrink-0" />
        <input
          ref={input}
          type="search"
          enterkeyhint="search"
          placeholder="Search tasks, projects and actions"
          aria-label="Search"
          autocomplete="off"
          value={needle()}
          onInput={(event) => setNeedle(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              const first = tasks()[0];
              if (first && text()) jump(first);
            }
          }}
        />
        <Show when={needle()}>
          <button type="button" class="search-clear" aria-label="Clear search" onClick={() => setNeedle('')}>
            <X size={16} />
          </button>
        </Show>
      </div>

      <Show when={text()}>
        <button
          type="button"
          class="search-row search-filter-row"
          onClick={() => {
            ui.setQuery(needle().trim());
            close();
          }}
        >
          <Filter size={18} class="text-text-muted" />
          <span class="min-w-0 flex-1 truncate">
            Filter the board by <b>“{needle().trim()}”</b>
          </span>
        </button>
      </Show>

      <Show when={store.target()?.slug}>
        <h3 class="board-section-label">{text() ? 'Tasks' : 'Recent tasks'}</h3>
        <div class="search-results" role="listbox" aria-label="Tasks">
          <For each={tasks()} fallback={<p class="text-sm text-text-dim py-3">No tasks match.</p>}>
            {(task) => (
              <button type="button" role="option" class="search-row" onClick={() => jump(task)}>
                <StatusGlyph status={task.status} size={16} />
                <span class="bcard-id shrink-0">{task.id}</span>
                <span class="min-w-0 flex-1 truncate" classList={{ derived: !task.title }}>
                  {displayTitle(task)}
                </span>
                <span class="search-kind">{laneLabel(task.status)}</span>
              </button>
            )}
          </For>
        </div>
      </Show>

      <Show when={shownCommands().length > 0}>
        <h3 class="board-section-label">Actions</h3>
        <div class="search-results">
          <For each={shownCommands()}>
            {(command) => (
              <button type="button" class="search-row" onClick={() => choose(command)}>
                <span class="search-icon">{command.icon}</span>
                <span class="min-w-0 flex-1 truncate">{command.label}</span>
                <Show when={command.hint}>
                  <span class="lane-pill-count">{command.hint}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>
    </BoardSheet>
  );
}
