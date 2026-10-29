/**
 * List view (UNI-130), the kanboard web UI's List.tsx on a phone: tasks
 * grouped by lane with sticky group headers, two-line rows (priority,
 * id, title / status, agent, deps, labels, age). Empty groups are left
 * out unless a scope (Review queue / Blocked) narrows to one lane.
 */
import { createSignal, For, Show } from 'solid-js';
import { Plus } from 'lucide-solid';
import { displayTitle, PRIORITY_LABEL, type Task } from '../../lib/board/api';
import { laneLayout, laneTasks, type LaneItem } from '../../lib/board/view';
import { relativeTime } from '../../lib/board/markdown';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { PriorityGlyph, StatusGlyph } from './Glyphs';
import { AgentChip, cardPressHandlers, DepTag, LabelTags, now, StaleTag } from './TaskCard';
import { CardSkeletons, PullIndicator, usePull } from './BoardLanes';

export function BoardList(props: {
  lanes: { id: string; label: string }[];
  onRefresh: () => void;
  refreshing: boolean;
  onArmed: () => void;
}) {
  const store = boardStore();
  const ui = boardUi();
  const [pull, setPull] = createSignal(0);
  const groups = () =>
    props.lanes.map((lane) => {
      const tasks = laneTasks(store.state.tasks, lane.id, ui.query(), ui.filters);
      return { lane, tasks, items: laneLayout(tasks, ui.display.chains) };
    });
  const nonEmpty = () => groups().filter((group) => group.tasks.length > 0 || ui.scope() !== 'all');
  const pullRef = usePull(props.onRefresh, setPull, props.onArmed);
  return (
    <div class="board-list-wrap">
      <PullIndicator distance={pull()} refreshing={props.refreshing} />
      <div class="board-list" ref={pullRef} role="list" aria-label="Tasks" data-testid="board-list">
        <Show when={!store.state.loaded && (store.state.loading || !store.state.error)}>
          <div class="px-4 pt-2 flex flex-col gap-2">
            <CardSkeletons count={6} />
          </div>
        </Show>
        <For each={nonEmpty()}>
          {(group) => (
            <section class="list-group" data-lane={group.lane.id} aria-label={`${group.lane.label}, ${group.tasks.length} tasks`}>
              <div class="list-group-head">
                <StatusGlyph status={group.lane.id} size={14} />
                <span>{group.lane.label}</span>
                <span class="lane-pill-count">{group.tasks.length}</span>
                <span class="flex-1" />
                <Show when={group.lane.id === 'backlog' || group.lane.id === 'todo'}>
                  <button
                    type="button"
                    class="ghost-icon-button"
                    aria-label={`Add to ${group.lane.label}`}
                    onClick={() => ui.openNewTask({ lane: group.lane.id as 'backlog' | 'todo' })}
                  >
                    <Plus size={18} />
                  </button>
                </Show>
              </div>
              <For each={group.items} fallback={<div class="list-empty">Nothing here.</div>}>
                {(item) => <ListRow item={item} />}
              </For>
            </section>
          )}
        </For>
        <Show when={store.state.loaded && nonEmpty().length === 0}>
          <div class="list-none">
            <span>No tasks match.</span>
            <button type="button" class="lane-empty-clear" onClick={() => ui.clearFilters()}>
              Clear filters
            </button>
          </div>
        </Show>
      </div>
    </div>
  );
}

function ListRow(props: { item: LaneItem }) {
  const task = (): Task => props.item.task;
  const hasTags = () =>
    !!task().run ||
    (task().status === 'in_progress' && !!task().staleness && task().staleness !== 'running') ||
    (task().deps ?? []).length > 0 ||
    (task().labels ?? []).length > 0;
  return (
    <div
      class="list-row"
      classList={{
        locked: (task().lockedBy ?? []).length > 0,
        final: ['done', 'cancelled', 'archived'].includes(task().status),
        cancelled: task().status === 'cancelled',
        [`chain-${props.item.chain}`]: props.item.chain !== 'single',
      }}
      role="listitem"
      tabindex="0"
      data-id={task().id}
      data-chain={props.item.chain}
      aria-label={`${task().id} ${displayTitle(task())}`}
      {...cardPressHandlers(task)}
    >
      <div class="list-row-line">
        <span class="list-row-prio" title={PRIORITY_LABEL[task().priority]}>
          <PriorityGlyph priority={task().priority} size={14} />
        </span>
        <span class="bcard-id">{task().id}</span>
        <span class="list-row-title" classList={{ derived: !task().title }}>
          {displayTitle(task())}
        </span>
        <span class="list-row-time" title={task().updated}>
          {relativeTime(task().updated, now()).replace(' ago', '')}
        </span>
      </div>
      <Show when={hasTags()}>
        <div class="list-row-tags">
          <AgentChip task={task()} />
          <StaleTag task={task()} />
          <DepTag task={task()} drawnParents={props.item.parents} />
          <LabelTags labels={task().labels ?? []} max={2} />
        </div>
      </Show>
    </div>
  );
}
