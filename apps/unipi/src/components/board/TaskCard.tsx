/**
 * Board card + its tag vocabulary (UNI-130), ported from the kanboard
 * web UI's Board.tsx `Card` and paint.tsx: id, label chips, agent chip
 * (pulse + elapsed), stale tag, title (derived titles italic), blocked
 * reason callout, optional excerpt, priority tag, dependency tag
 * (ready / waiting / locked; dep ids jump to that card).
 *
 * Tap → `boardActions().openTask(task)`; long-press (contextmenu, as on
 * the Sessions cards) → `boardActions().openCardMenu(task, rect)`.
 */
import { clockNow } from '../../lib/clock';
import { createRoot, createSignal, For, Show } from 'solid-js';
import { Link2, Lock } from 'lucide-solid';
import { displayTitle, PRIORITY_LABEL, type Task } from '../../lib/board/api';
import { elapsed, excerptOf, hue, type ChainPos } from '../../lib/board/view';
import { plainText } from '../../lib/board/markdown';
import { boardActions, openDependency } from '../../lib/board/actions';
import { menuAnchor } from '../../lib/layout';
import { PriorityGlyph } from './Glyphs';

const FINAL = new Set(['done', 'cancelled', 'archived']);

/** Ticks every 15 s so "agent 4m" stays current (the app's shared clock: one timer, only while a card reads it). */
export function now(): number {
  return clockNow(15_000);
}

export function PriorityTag(props: { priority: string }) {
  return (
    <Show when={props.priority && props.priority !== 'none'}>
      <span class={`btag prio prio-${props.priority}`} title={`Priority: ${PRIORITY_LABEL[props.priority]}`}>
        <PriorityGlyph priority={props.priority} size={12} />
        <span>{PRIORITY_LABEL[props.priority]}</span>
      </span>
    </Show>
  );
}

/** locked: a dep is still in Backlog; waiting: deps in flight; ready: every dep reached the gate. */
export function depState(task: Task, drawnParents: string[] = []): { deps: string[]; locked: string[]; waiting: string[] } {
  const drawn = new Set(drawnParents);
  const all = task.deps ?? [];
  const deps = all.filter((dep) => !drawn.has(dep) || all.length > 1);
  return {
    deps,
    locked: (task.lockedBy ?? []).filter((dep) => deps.includes(dep)),
    waiting: (task.waitingFor ?? []).filter((dep) => deps.includes(dep)),
  };
}

export function DepTag(props: { task: Task; drawnParents?: string[] }) {
  const info = () => depState(props.task, props.drawnParents);
  const kind = () => (info().locked.length > 0 ? 'locked' : info().waiting.length > 0 ? 'waiting' : 'ready');
  const title = (): string => {
    const { deps, locked, waiting } = info();
    return locked.length > 0
      ? `Locked: ${locked.join(', ')} ${locked.length === 1 ? 'is' : 'are'} still in Backlog`
      : waiting.length > 0
        ? `Waiting on ${waiting.join(', ')}`
        : `After ${deps.join(', ')} (ready)`;
  };
  return (
    <Show when={info().deps.length > 0}>
      <span class={`btag dep ${kind()}`} title={title()} data-dep-state={kind()}>
        <Show when={kind() !== 'ready'} fallback={<Link2 size={12} />}>
          <Lock size={11} />
        </Show>
        <span>
          after{' '}
          <For each={info().deps}>
            {(dep, index) => (
              <>
                <Show when={index() > 0}>, </Show>
                <button
                  type="button"
                  class="dep-id"
                  aria-label={`Open ${dep}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    openDependency(dep);
                  }}
                >
                  {dep}
                </button>
              </>
            )}
          </For>
          <Show when={kind() === 'locked'}> · not scheduled</Show>
        </span>
      </span>
    </Show>
  );
}

export function LabelTags(props: { labels: string[]; max?: number }) {
  const shown = () => props.labels.slice(0, props.max ?? 3);
  return (
    <>
      <For each={shown()}>
        {(label) => (
          <span class="btag label" style={{ '--label-hue': hue(label) }}>
            <span>{label}</span>
          </span>
        )}
      </For>
      <Show when={props.labels.length > shown().length}>
        <span class="btag label more">+{props.labels.length - shown().length}</span>
      </Show>
    </>
  );
}

export function AgentChip(props: { task: Task }) {
  return (
    <Show when={props.task.run}>
      <span class="agent-chip" title={`Claimed by session ${props.task.run?.session ?? '?'}`}>
        <span class="pulse" aria-hidden="true" />
        agent
        <span class="time">{elapsed(props.task.run?.started, now())}</span>
      </span>
    </Show>
  );
}

export function StaleTag(props: { task: Task }) {
  return (
    <Show when={props.task.status === 'in_progress' && props.task.staleness && props.task.staleness !== 'running'}>
      <span class="btag stale">stale</span>
    </Show>
  );
}

/** Long-press: Android WebView fires contextmenu; the press shrink comes from :active. */
export function cardPressHandlers(
  task: () => Task,
  onLongPress?: () => void,
): { onClick: () => void; onContextMenu: (event: MouseEvent) => void; onKeyDown: (event: KeyboardEvent) => void } {
  return {
    onClick: () => boardActions().openTask(task()),
    onContextMenu: (event: MouseEvent) => {
      event.preventDefault();
      onLongPress?.();
      boardActions().openCardMenu(task(), menuAnchor(event, event.currentTarget as HTMLElement));
    },
    onKeyDown: (event: KeyboardEvent) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        boardActions().openTask(task());
      }
    },
  };
}

export function TaskCard(props: {
  task: Task;
  chain?: ChainPos;
  parents?: string[];
  excerpt: boolean;
  compact: boolean;
  onLongPress?: () => void;
  /** Desktop / web pointer drag (./cardDrag.ts, lib/board/dnd.ts); absent on the phone. */
  drag?: { onPointerDown: (event: PointerEvent) => void; dragging: boolean; enabled: boolean };
}) {
  const task = () => props.task;
  const final = () => FINAL.has(task().status);
  const blocked = () => (task().status === 'blocked' && task().blockedReason?.text ? plainText(task().blockedReason!.text) : '');
  const excerpt = () => (props.excerpt && !props.compact ? excerptOf(task()) : '');
  const chain = () => props.chain ?? 'single';
  return (
    <article
      class="bcard"
      classList={{
        final: final(),
        cancelled: task().status === 'cancelled',
        running: !!task().run,
        locked: (task().lockedBy ?? []).length > 0,
        compact: props.compact,
        dragging: !!props.drag?.dragging,
        [`chain-${chain()}`]: chain() !== 'single',
      }}
      data-draggable={props.drag?.enabled ? '' : undefined}
      onPointerDown={(event) => props.drag?.enabled && props.drag.onPointerDown(event)}
      onDragStart={(event) => props.drag && event.preventDefault()}
      data-id={task().id}
      data-chain={chain()}
      tabindex="0"
      role="button"
      aria-label={`${task().id} ${displayTitle(task())}`}
      {...cardPressHandlers(task, props.onLongPress)}
    >
      <div class="bcard-top">
        <span class="bcard-id">{task().id}</span>
        <LabelTags labels={task().labels ?? []} max={2} />
        <span class="flex-1" />
        <AgentChip task={task()} />
        <StaleTag task={task()} />
      </div>
      <div class="bcard-title" classList={{ derived: !task().title }}>
        {displayTitle(task())}
      </div>
      <Show when={blocked()}>
        <div class="bcard-blocked">{blocked()}</div>
      </Show>
      <Show when={excerpt()}>
        <div class="bcard-excerpt">{excerpt()}</div>
      </Show>
      <Show when={(task().priority && task().priority !== 'none') || (task().deps ?? []).length > 0}>
        <div class="bcard-meta">
          <PriorityTag priority={task().priority} />
          <DepTag task={task()} drawnParents={props.parents} />
        </div>
      </Show>
    </article>
  );
}
