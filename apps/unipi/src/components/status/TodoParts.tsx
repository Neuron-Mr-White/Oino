/**
 * The agent's todo list (the `todowrite` tool's latest snapshot of a chat) in
 * its three places, all fed by the same pure helpers in `lib/status/plan.ts`:
 *
 *   TodoList      the checklist rows (Plan section, Todo section, capsule drawer)
 *   TodoSection   the side pane's "Todo · 2/6" section (top of the Graph tab)
 *   TodoCapsule   the desktop pill above the composer ("2/6 · item") that opens
 *                 the same checklist in a drawer; hidden while the side pane
 *                 already shows a Todo section (`todoSectionShown`)
 */
import { createEffect, createMemo, createSignal, on, onCleanup, For, Show } from 'solid-js';
import { Check, ChevronUp, Circle, CircleX, ListChecks } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { panes } from '../../lib/panes';
import { sessions } from '../../lib/session/store';
import { latestTodos, todoCapsuleLabel, todoSummary, type TodoItem, type TodoSummary } from '../../lib/status/plan';
import { currentWorkspace } from '../shell/Workspace';
import './status.css';

const TODO_GLYPH: Record<Exclude<TodoItem['status'], 'in_progress'>, typeof Check> = {
  pending: Circle,
  completed: Check,
  cancelled: CircleX,
};

/** One checklist: hollow circle · pulsing accent dot (in progress) · check (done, struck) · x (cancelled). */
export function TodoList(props: { todos: readonly TodoItem[] }) {
  return (
    <ul class="todo-list" data-testid="todo-list">
      <For each={props.todos}>
        {(todo) => {
          const Glyph = todo.status === 'in_progress' ? undefined : TODO_GLYPH[todo.status];
          return (
            <li class="todo-item" data-status={todo.status} data-testid="todo-item" aria-current={todo.status === 'in_progress' ? 'step' : undefined}>
              <span class="todo-glyph" aria-hidden="true">
                {Glyph ? <Glyph size={13} /> : <span class="todo-pulse" />}
              </span>
              <span class="todo-text">{todo.content}</span>
              <Show when={todo.priority === 'high'}>
                <span class="todo-priority">high</span>
              </Show>
            </li>
          );
        }}
      </For>
    </ul>
  );
}

/** The chat's latest todo snapshot (entries + a live `todowrite` call), reactive. */
export function useTodos(hostId: () => string, sessionId: () => string): () => TodoItem[] {
  const s = createMemo(() => sessions().ensure(hostId(), sessionId()));
  createEffect(on(s, (handle) => onCleanup(handle.acquire())));
  return createMemo(() => latestTodos(s().state.entries, s().state.liveTools));
}

function ProgressBar(props: { summary: TodoSummary }) {
  return (
    <div class="status-bar" role="progressbar" aria-label="Todos" aria-valuemin={0} aria-valuemax={props.summary.total} aria-valuenow={props.summary.done} data-testid="todo-bar">
      <span style={{ width: `${Math.min(100, (props.summary.done / props.summary.total) * 100)}%` }} data-mode="todo" />
    </div>
  );
}

/** The side pane's Todo section: "Todo · done/total", the checklist, a progress bar. Hidden without todos. */
export function TodoSection(props: { hostId: string; sessionId: string }) {
  const todos = useTodos(() => props.hostId, () => props.sessionId);
  const summary = createMemo(() => todoSummary(todos()));
  return (
    <Show when={todos().length > 0}>
      <section class="status-section" data-testid="todo-section" aria-label="Todo">
        <div class="status-section-head">
          <span class="status-section-title" data-testid="todo-section-title">
            <ListChecks size={13} />
            {summary() ? `Todo · ${summary()!.done}/${summary()!.total}` : 'Todo'}
          </span>
        </div>
        <div class="plan-body">
          <TodoList todos={todos()} />
          <Show when={summary()}>{(m) => <ProgressBar summary={m()} />}</Show>
        </div>
      </section>
    </Show>
  );
}

/** A Todo section is on screen in the side pane (the Graph or Plan tab, pane pushing the chat): the capsule steps aside. */
export function todoSectionShown(): boolean {
  const L = layout();
  if (!L.desktop() || !L.sideOpen() || L.sideMode() === 'overlay') return false;
  const ws = currentWorkspace();
  if (!ws) return false;
  const tab = panes().active(ws.scope, 'side', ws.view);
  if (tab?.kind !== 'status') return false;
  const section = tab.params?.section;
  return section === undefined || section === 'lh' || section === 'plan';
}

/** The pill above the composer; click toggles the checklist drawer (Esc / outside click close it). */
export function TodoCapsule(props: { hostId: string; sessionId: string }) {
  const todos = useTodos(() => props.hostId, () => props.sessionId);
  const summary = createMemo(() => todoSummary(todos()));
  const [open, setOpen] = createSignal(false);
  let root: HTMLDivElement | undefined;
  let btn: HTMLButtonElement | undefined;

  createEffect(() => {
    if (!open()) return;
    const onDown = (e: Event) => {
      if (root && !root.contains(e.target as Node)) setOpen(false);
    };
    // Capture + stop: Esc closes the drawer without also stopping a running turn.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      btn?.focus();
    };
    document.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => {
      document.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
    });
  });

  return (
    <Show when={summary() && !todoSectionShown()}>
      <div class="todo-capsule" ref={root} data-testid="todo-capsule" data-open={open() ? '' : undefined}>
        <Show when={open()}>
          <div class="todo-drawer" role="dialog" aria-label="Todo" data-testid="todo-drawer">
            <div class="todo-drawer-head">
              <ListChecks size={13} />
              <span>{`Todo · ${summary()!.done}/${summary()!.total}`}</span>
            </div>
            <TodoList todos={todos()} />
          </div>
        </Show>
        <button
          type="button"
          ref={btn}
          class="todo-capsule-btn"
          data-state={summary()!.allDone ? 'done' : summary()!.current?.status}
          aria-expanded={open()}
          aria-label={`Todo list, ${todoCapsuleLabel(summary()!)}`}
          data-testid="todo-capsule-btn"
          onClick={() => setOpen((v) => !v)}
        >
          <span class="todo-capsule-glyph" aria-hidden="true">
            <Show when={summary()!.current?.status === 'in_progress'} fallback={summary()!.allDone ? <Check size={13} /> : <ListChecks size={13} />}>
              <span class="todo-pulse" />
            </Show>
          </span>
          <span class="todo-capsule-label truncate" data-testid="todo-capsule-label">{todoCapsuleLabel(summary()!)}</span>
          <ChevronUp size={13} class="todo-capsule-chev" aria-hidden="true" />
        </button>
      </div>
    </Show>
  );
}
