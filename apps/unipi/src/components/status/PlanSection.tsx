/**
 * The status pane's Plan & todos section (WP-A16, RULINGS Z-43): the
 * workflow plan state (plan mode, permission mode, plan file — sticky
 * `unipi:workflow:status` topic + `unipi:plan-mode` entries) and the todo
 * tool's checklist (the latest `todowrite` call). Plan **approval** stays a
 * dialog (the dock's dialog panel); no inline todo cards (§3.10). Hidden
 * while there is neither plan mode nor todos — no empty filler.
 */
import { createEffect, createMemo, on, onCleanup, Show } from 'solid-js';
import { Check } from 'lucide-solid';
import { sessions } from '../../lib/session/store';
import {
  latestPlanEntry,
  latestTodos,
  planFileLabel,
  readSessionWorkflowStatus,
  sessionWorkflowTopic,
  todoProgress,
} from '../../lib/status/plan';
import { TodoList } from './TodoParts';
import './status.css';

export function PlanSection(props: { hostId: string; sessionId: string }) {
  const s = createMemo(() => sessions().ensure(props.hostId, props.sessionId));
  createEffect(
    on(s, (handle) => {
      const release = handle.acquire();
      handle.subscribeTopics([sessionWorkflowTopic(props.sessionId)]);
      onCleanup(release);
    }),
  );
  const workflow = () => readSessionWorkflowStatus(s().state.topics, props.sessionId);
  const planEntry = () => latestPlanEntry(s().state.entries);
  const todos = createMemo(() => latestTodos(s().state.entries, s().state.liveTools));
  const progress = () => todoProgress(todos());
  const planOn = () => workflow()?.planMode ?? planEntry()?.active ?? false;
  const show = () => planOn() || todos().length > 0;

  return (
    <Show when={show()}>
      <section class="status-section" data-testid="plan-section" aria-label="Plan and todos">
        <div class="status-section-head">
          <span class="status-section-title">
            <Check size={13} /> Plan &amp; todos
          </span>
          <Show when={todos().length > 0}>
            <span class="status-chip" data-testid="todo-count">
              {progress().done}/{progress().total}
            </span>
          </Show>
        </div>
        <div class="plan-body">
          <Show when={planOn()}>
            <p class="plan-mode-line" data-testid="plan-mode">
              <span class="status-chip" data-phase="active">Plan mode</span>
              <Show when={planFileLabel(planEntry()?.planFile)}>{(f) => <span class="status-dim truncate" title={planEntry()?.planFile}>{f()}</span>}</Show>
              <Show when={workflow()?.permissionMode}>{(p) => <span class="status-dim truncate">{p()}</span>}</Show>
            </p>
          </Show>
          <Show when={todos().length > 0}>
            <TodoList todos={todos()} />
            <Show when={progress().total > 0}>
              <div class="status-bar" role="progressbar" aria-label="Todos" aria-valuemin={0} aria-valuemax={progress().total} aria-valuenow={progress().done} data-testid="todo-bar">
                <span style={{ width: `${Math.min(100, (progress().done / progress().total) * 100)}%` }} data-mode="todo" />
              </div>
            </Show>
          </Show>
        </div>
      </section>
    </Show>
  );
}
