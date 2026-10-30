/**
 * Board lanes on a phone (UNI-130): a tappable, scrollable lane strip
 * with counts on top, and the lanes themselves as full-width pages in a
 * horizontal scroll-snap pager (one swipe = one lane; every page keeps
 * its own vertical scroll position). Each page has the lane's one-line
 * "about" text + quick actions (web lane header), its cards, an empty
 * state, and pull-to-refresh at the top.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Archive, Check, EllipsisVertical, EyeOff, Info, Plus, RefreshCw } from 'lucide-solid';
import { laneLabel, type Rules } from '../../lib/board/api';
import { laneCount, laneLayout, laneTasks } from '../../lib/board/view';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { boardActions } from '../../lib/board/actions';
import { attachPullToRefresh, PULL_THRESHOLD } from '../../lib/board/pull';
import { desktopShell } from '../../lib/layout';
import { boardDrag, FINAL_LANES, dropAllowed, planDrop } from '../../lib/board/dnd';
import { pressCard } from './cardDrag';
import { dropTask } from '../../lib/board/taskActions';
import { laneLabel as labelOf, type Task } from '../../lib/board/api';
import './board-desktop.css';
import { StatusGlyph } from './Glyphs';
import { TaskCard } from './TaskCard';
import { AnchoredMenu, MenuRow, MenuSeparator } from './AnchoredMenu';

export const LANE_INFO: Record<string, (rules: Rules) => string> = {
  backlog: () => 'Ideas and later work. Never run automatically. Move to Todo when a task is ready.',
  todo: (rules) =>
    `Ready to work. An agent picks the next task by priority, then the order here. A task waits until every task it runs after reaches ${rules.chainGate === 'done' ? 'Done' : 'In Review'}.`,
  in_progress: (rules) =>
    `Being worked by an agent session (at most ${rules.maxSessions ?? 2} per project). Locked to its session while it runs.`,
  blocked: () => 'The agent needs something from you. Read the reason, reply with a note, then move it back to Todo.',
  in_review: () => 'The agent finished. Check the activity, then move it to Done, or back to Todo with a note on what to change.',
  done: () => 'Accepted by you. Summarize & archive, or archive without a summary, from the ⋮ menu.',
  cancelled: () => (desktopShell() ? 'Dropped work. Right-click a card to recreate it.' : 'Dropped work. Long-press a card to recreate it.'),
  archived: () => 'Archived tasks, kept for the record.',
};

export const LANE_EMPTY: Record<string, string> = {
  backlog: 'Capture ideas here',
  todo: 'Ready work lands here',
  in_progress: 'No agent is working',
  in_review: 'Nothing waiting for review',
  blocked: 'Nothing blocked',
  done: 'Nothing finished yet',
  cancelled: 'Nothing cancelled',
  archived: 'Archive is empty',
};

type LaneDef = { id: string; label: string };

/** Lane strip: one pill per visible lane (glyph, name, count); tap = go to that page. */
export function LaneStrip(props: { lanes: LaneDef[]; active: string; onPick: (lane: string) => void; loading: boolean }) {
  const store = boardStore();
  const ui = boardUi();
  let strip: HTMLDivElement | undefined;
  const filtered = () => ui.query().trim().length > 0 || ui.filters.priorities.length + ui.filters.labels.length > 0 || ui.filters.running || ui.filters.waiting;
  const count = (lane: string) =>
    filtered() ? laneTasks(store.state.tasks, lane, ui.query(), ui.filters).length : laneCount(store.state.tasks, lane);
  createEffect(
    on(
      () => props.active,
      (active) => {
        const pill = strip?.querySelector<HTMLElement>(`[data-lane="${active}"]`);
        if (!pill || !strip) return;
        // Centre the active pill without scrolling the page.
        const target = pill.offsetLeft - (strip.clientWidth - pill.offsetWidth) / 2;
        strip.scrollTo?.({ left: Math.max(0, target), behavior: 'smooth' });
      },
    ),
  );
  return (
    <div class="lane-strip" ref={strip} role="tablist" aria-label="Lanes">
      <For each={props.lanes}>
        {(lane) => (
          <button
            type="button"
            role="tab"
            class="lane-pill"
            data-lane={lane.id}
            aria-selected={props.active === lane.id}
            aria-controls={`lane-page-${lane.id}`}
            onClick={() => props.onPick(lane.id)}
          >
            <StatusGlyph status={lane.id} size={14} />
            <span>{lane.label}</span>
            <Show when={!props.loading || store.state.loaded} fallback={<span class="lane-pill-count skeleton-count" />}>
              <span class="lane-pill-count">{count(lane.id)}</span>
            </Show>
          </button>
        )}
      </For>
    </div>
  );
}

/** Pull indicator shared by the pages / list. */
export function PullIndicator(props: { distance: number; refreshing: boolean }) {
  return (
    <div
      class="pull-indicator"
      classList={{ visible: props.distance > 4 || props.refreshing, armed: props.distance >= PULL_THRESHOLD }}
      style={{ transform: `translate(-50%, ${props.refreshing ? 56 : Math.min(props.distance, 96) - 40}px)` }}
      aria-hidden={!props.refreshing}
      role="status"
      aria-label={props.refreshing ? 'Refreshing' : undefined}
    >
      <RefreshCw
        size={18}
        class={props.refreshing ? 'refresh-spinning' : ''}
        style={{ transform: props.refreshing ? undefined : `rotate(${props.distance * 3}deg)` }}
      />
    </div>
  );
}

/** Hooks a scroller up to pull-to-refresh; returns the ref callback. */
export function usePull(onRefresh: () => void, setDistance: (n: number) => void, onArmed: () => void) {
  return (el: HTMLElement) => {
    onMount(() => onCleanup(attachPullToRefresh(el, { onPull: setDistance, onRefresh, onArmed })));
  };
}

export function LanePager(props: {
  lanes: LaneDef[];
  active: string;
  onActiveChange: (lane: string) => void;
  onRefresh: () => void;
  refreshing: boolean;
  onHaptic: (kind: 'tap' | 'tick') => void;
  onHideLane: (lane: string) => void;
  /** Exposes `scrollToLane` so the strip / highlight can drive the pager. */
  ref?: (api: { scrollToLane: (lane: string, smooth?: boolean) => void; pageFor: (lane: string) => HTMLElement | undefined }) => void;
}) {
  let pager: HTMLDivElement | undefined;
  const [pull, setPull] = createSignal(0);
  let programmatic = false;
  let settleTimer: ReturnType<typeof setTimeout> | undefined;

  const indexOf = (lane: string) => props.lanes.findIndex((candidate) => candidate.id === lane);
  function scrollToLane(lane: string, smooth = true) {
    const index = indexOf(lane);
    if (!pager || index < 0) return;
    // Desktop shell (UNI-117): lanes are side-by-side columns, not pages.
    if (desktopShell()) {
      pager.querySelector<HTMLElement>(`[data-lane-page="${lane}"]`)?.scrollIntoView?.({ inline: 'nearest', block: 'nearest', behavior: smooth ? 'smooth' : 'auto' });
      return;
    }
    const left = index * pager.clientWidth;
    if (Math.abs(pager.scrollLeft - left) < 2) return;
    // Adjacent lane: animate; a far jump is instant (a long smooth scroll
    // through every lane in between reads as lag).
    if (Math.abs(pager.scrollLeft - left) > pager.clientWidth * 1.5) smooth = false;
    programmatic = true;
    pager.scrollTo?.({ left, behavior: smooth ? 'smooth' : 'auto' });
    if (!smooth) {
      pager.scrollLeft = left;
      programmatic = false;
    }
  }
  function settle() {
    if (!pager || pager.clientWidth === 0 || desktopShell()) return;
    programmatic = false;
    const index = Math.round(pager.scrollLeft / pager.clientWidth);
    const lane = props.lanes[Math.max(0, Math.min(index, props.lanes.length - 1))];
    if (lane && lane.id !== props.active) {
      props.onHaptic('tick');
      props.onActiveChange(lane.id);
    }
  }
  const onScroll = () => {
    clearTimeout(settleTimer);
    // `scrollend` settles immediately where supported; this is the fallback.
    settleTimer = setTimeout(settle, 140);
  };

  onMount(() => {
    props.ref?.({ scrollToLane, pageFor: (lane) => pager?.querySelector<HTMLElement>(`[data-lane-page="${lane}"]`) ?? undefined });
    pager?.addEventListener('scrollend', () => {
      clearTimeout(settleTimer);
      settle();
    });
    // Land on the remembered lane without animating.
    requestAnimationFrame(() => scrollToLane(props.active, false));
    const resize = () => scrollToLane(props.active, false);
    window.addEventListener('resize', resize);
    onCleanup(() => window.removeEventListener('resize', resize));
  });
  // Lane set changed (backlog toggled, column hidden): keep the active page in view.
  createEffect(
    on(
      () => props.lanes.map((lane) => lane.id).join(','),
      // Desktop: columns stay where they are (a drag adds/removes Backlog).
      () => !desktopShell() && requestAnimationFrame(() => scrollToLane(props.active, false)),
      { defer: true },
    ),
  );
  onCleanup(() => clearTimeout(settleTimer));

  // Card drag on the desktop columns (pointer events, ./cardDrag.ts): the
  // drop is planned against the target lane's stored (unfiltered) order.
  const dnd = boardDrag();
  const store = boardStore();
  function dropCard(task: Task, lane: string, before: string | null) {
    const ids = laneTasks(store.state.tasks, lane, '', { priorities: [], labels: [], running: false, waiting: false }).map((item) => item.id);
    void dropTask(task, planDrop(store.state.rules, task, lane, before, ids, labelOf));
  }

  return (
    <div class="lane-pager-wrap" data-dragging={dnd.drag() ? '' : undefined}>
      <PullIndicator distance={pull()} refreshing={props.refreshing} />
      <div class="lane-pager" ref={pager} onScroll={() => !programmatic && onScroll()} data-testid="lane-pager">
        <For each={props.lanes}>
          {(lane) => (
            <LanePage
              lane={lane}
              active={props.active === lane.id}
              onRefresh={props.onRefresh}
              setPull={setPull}
              onArmed={() => props.onHaptic('tick')}
              onHide={() => props.onHideLane(lane.id)}
              pager={() => pager}
              onDropCard={dropCard}
            />
          )}
        </For>
      </div>
    </div>
  );
}

function LanePage(props: {
  lane: LaneDef;
  active: boolean;
  onRefresh: () => void;
  setPull: (n: number) => void;
  onArmed: () => void;
  onHide: () => void;
  pager: () => HTMLElement | undefined;
  onDropCard: (task: Task, lane: string, before: string | null) => void;
}) {
  const store = boardStore();
  const ui = boardUi();
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  const tasks = createMemo(() => laneTasks(store.state.tasks, props.lane.id, ui.query(), ui.filters));
  const items = createMemo(() => laneLayout(tasks(), ui.display.chains));
  const total = () => laneCount(store.state.tasks, props.lane.id);
  const info = () => LANE_INFO[props.lane.id]?.(store.state.rules) ?? '';
  const canAdd = () => props.lane.id === 'backlog' || props.lane.id === 'todo';
  const pullRef = usePull(props.onRefresh, props.setPull, props.onArmed);

  // ── drag & drop (desktop shell only: lanes are columns there) ──────────
  const dnd = boardDrag();
  const dragOn = () => desktopShell();
  const dragging = () => dnd.drag();
  const refuses = () => {
    const current = dragging();
    return !!current && !dropAllowed(store.state.rules, current.task, props.lane.id);
  };
  const hovered = () => dragging()?.lane === props.lane.id;
  function dragHandlers(task: () => Task) {
    return {
      // A running card can be pressed: the drag then says why it won't
      // move (dragBlocked). Final lanes just don't drag.
      enabled: dragOn() && !FINAL_LANES.has(task().status),
      dragging: dragging()?.task.id === task().id,
      onPointerDown: (event: PointerEvent) =>
        pressCard(event, event.currentTarget as HTMLElement, {
          task,
          pager: props.pager,
          onDrop: props.onDropCard,
          onBlocked: (message) => ui.toast(message, 'warning'),
        }),
    };
  }
  const showLine = (before: string | null) => {
    const current = dragging();
    return !!current && hovered() && !refuses() && current.before === before && !(before === null && current.task.status === props.lane.id && items().at(-1)?.task.id === current.task.id);
  };

  return (
    <section
      class="lane-page"
      id={`lane-page-${props.lane.id}`}
      data-lane-page={props.lane.id}
      role="tabpanel"
      aria-label={`${props.lane.label}, ${total()} ${total() === 1 ? "task" : "tasks"}`}
      data-active={props.active ? '' : undefined}
      data-drop={dragging() ? (refuses() ? 'refused' : hovered() && dragging()!.task.status !== props.lane.id ? 'ok' : 'allowed') : undefined}
      ref={pullRef}
    >
      <Show
        when={dragOn()}
        fallback={
      <div class="lane-head">
        <p class="lane-info">{info()}</p>
        <Show when={props.lane.id === 'in_review' && total() > 0}>
          <button type="button" class="lane-quick" onClick={() => boardActions().laneAction('in_review', 'done-all')}>
            <Check size={15} /> Done all
          </button>
        </Show>
        <button
          type="button"
          class="ghost-icon-button lane-more"
          aria-label={`${props.lane.label} options`}
          aria-expanded={menu() !== undefined}
          onClick={(event) => setMenu(event.currentTarget.getBoundingClientRect())}
        >
          <EllipsisVertical size={18} />
        </button>
      </div>
        }
      >
        {/* Desktop column head (web kanboard): name · count · ⓘ … quick action · ⋯ */}
        <header class="lane-head lane-head-desk">
          <span class="lane-title">
            <StatusGlyph status={props.lane.id} size={13} />
            <span>{props.lane.label}</span>
          </span>
          <span class="lane-count tabular-nums">{tasks().length === total() ? total() : `${tasks().length} of ${total()}`}</span>
          <Show when={info()}>
            <span class="lane-info-dot" role="img" aria-label={info()} title={info()}>
              <Info size={14} />
            </span>
          </Show>
          <span class="lane-spacer" />
          <Show when={props.lane.id === 'in_review' && total() > 0}>
            <button type="button" class="lane-quick" onClick={() => boardActions().laneAction('in_review', 'done-all')}>
              <Check size={13} /> Done all
            </button>
          </Show>
          <button
            type="button"
            class="ghost-icon-button lane-more"
            aria-label={`${props.lane.label} options`}
            aria-expanded={menu() !== undefined}
            onClick={(event) => setMenu(event.currentTarget.getBoundingClientRect())}
          >
            <EllipsisVertical size={16} />
          </button>
        </header>
      </Show>

      <div class="lane-cards">
        <Show when={!store.state.loaded && (store.state.loading || !store.state.error)}>
          <CardSkeletons />
        </Show>
        <For each={items()}>
          {(item) => (
            <>
              <Show when={showLine(item.task.id) && dragging()?.task.id !== item.task.id}>
                <div class="drop-line" aria-hidden="true" />
              </Show>
              <TaskCard
                task={item.task}
                chain={item.chain}
                parents={item.parents}
                excerpt={ui.display.excerpt}
                compact={ui.display.density === 'compact'}
                drag={dragOn() ? dragHandlers(() => item.task) : undefined}
              />
            </>
          )}
        </For>
        <Show when={showLine(null)}>
          <div class="drop-line" aria-hidden="true" />
        </Show>
        <Show when={store.state.loaded && tasks().length === 0 && !(dragging() && hovered())}>
          <LaneEmpty lane={props.lane.id} filteredOut={total() > 0} />
        </Show>
        <Show when={canAdd() && store.state.loaded}>
          <button type="button" class="lane-add" onClick={() => ui.openNewTask({ lane: props.lane.id as 'backlog' | 'todo' })}>
            <Plus size={16} /> Add task
          </button>
        </Show>
      </div>

      <Show when={menu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} alignEnd width={288} label={`${props.lane.label} options`} onDismiss={() => setMenu(undefined)}>
            <Show when={dragOn() && info()}>
              <p class="lane-menu-info">{info()}</p>
            </Show>
            <LaneMenuItems lane={props.lane} total={total()} close={() => setMenu(undefined)} onHide={props.onHide} />
          </AnchoredMenu>
        )}
      </Show>
    </section>
  );
}

function LaneMenuItems(props: { lane: LaneDef; total: number; close: () => void; onHide: () => void }) {
  const ui = boardUi();
  const run = (fn: () => void) => () => {
    props.close();
    fn();
  };
  return (
    <>
      <Show when={props.lane.id === 'done'}>
        <MenuRow icon={<Archive size={18} />} label={`Archive all (${props.total})`} disabled={props.total === 0} onSelect={run(() => boardActions().laneAction('done', 'archive-lane'))} />
        <MenuSeparator />
      </Show>
      <Show when={props.lane.id === 'in_review'}>
        <MenuRow icon={<Check size={18} />} label={`Move all (${props.total}) to Done`} disabled={props.total === 0} onSelect={run(() => boardActions().laneAction('in_review', 'done-all'))} />
        <MenuRow icon={<Archive size={18} />} label={`Archive all (${props.total})`} disabled={props.total === 0} onSelect={run(() => boardActions().laneAction('in_review', 'archive-lane'))} />
        <MenuSeparator />
      </Show>
      <Show when={props.lane.id === 'backlog' || props.lane.id === 'todo'}>
        <MenuRow icon={<Plus size={18} />} label={`Add to ${laneLabel(props.lane.id)}`} onSelect={run(() => ui.openNewTask({ lane: props.lane.id as 'backlog' | 'todo' }))} />
      </Show>
      <MenuRow icon={<EyeOff size={18} />} label="Hide column" onSelect={run(props.onHide)} />
    </>
  );
}

export function CardSkeletons(props: { count?: number }) {
  return (
    <For each={Array.from({ length: props.count ?? 4 })}>
      {(_, index) => <div class="bcard-skeleton skeleton" style={{ height: `${index() % 2 ? 92 : 112}px` }} />}
    </For>
  );
}

export function LaneEmpty(props: { lane: string; filteredOut: boolean }): JSX.Element {
  const ui = boardUi();
  return (
    <div class="lane-empty">
      <StatusGlyph status={props.lane} size={22} />
      <span>{props.filteredOut ? 'No tasks here match the filter' : (LANE_EMPTY[props.lane] ?? 'Nothing here')}</span>
      <Show when={props.filteredOut}>
        <button type="button" class="lane-empty-clear" onClick={() => ui.clearFilters()}>
          Clear filters
        </button>
      </Show>
    </div>
  );
}
