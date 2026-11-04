/**
 * Shared long-horizon building blocks ([lh-ui]): the status chip, the segmented progress bar,
 * the node row/card, the connected event timeline. Graph, swarm, goal and ralph views all
 * compose these so they read as one design system.
 */
import { For, Show, type JSX } from 'solid-js';
import { Ban, CircleCheck, CircleX, Clock, ExternalLink, Hourglass, Loader2, Replace, RotateCcw, Square, TriangleAlert, Wrench, Timer, CircleDollarSign } from 'lucide-solid';
import { prettyModelId } from '../../lib/model/displayName';
import {
  STATUS_META,
  eventTitle,
  progressSegments,
  shortProfile,
  workElapsedMs,
  type LhGraphCounts,
  type LhStatus,
  type LhTimelineEntry,
  type LhWork,
} from '../../lib/status/lhGraph';
import { clockText, durationText, startedText } from './lhProgress';

const ICON: Record<LhStatus, (p: { size: number }) => JSX.Element> = {
  queued: (p) => <Clock size={p.size} />,
  running: (p) => <Loader2 size={p.size} class="lh-spin" data-testid="lh-node-spinner" />,
  blocked: (p) => <Hourglass size={p.size} />,
  errored: (p) => <TriangleAlert size={p.size} />,
  completed: (p) => <CircleCheck size={p.size} />,
  failed: (p) => <CircleX size={p.size} />,
  stopped: (p) => <Square size={p.size} />,
  cancelled: (p) => <Ban size={p.size} />,
  superseded: (p) => <Replace size={p.size} />,
};

/** Status as a coloured chip with icon (running animated). */
export function StatusChip(props: { status: LhStatus; size?: 'sm' | 'md' }) {
  const meta = () => STATUS_META[props.status];
  const iconSize = () => (props.size === 'sm' ? 11 : 12);
  return (
    <span class="lh-chip lh-status" data-status={props.status} data-tone={meta().tone} data-size={props.size ?? 'md'} data-testid="lh-status-chip">
      {ICON[props.status]({ size: iconSize() })}
      <span>{meta().label}</span>
    </span>
  );
}

/** Profile / model chips of a node (distinct small chips). */
export function RunChips(props: { work: Pick<LhWork, 'profile' | 'model' | 'thinking'> }) {
  return (
    <>
      <Show when={props.work.profile}>
        {(p) => (
          <span class="lh-chip lh-chip-profile" title={`Profile: ${p()}`} data-testid="lh-chip-profile">
            {shortProfile(p())}
          </span>
        )}
      </Show>
      <Show when={props.work.model}>
        {(m) => (
          <span class="lh-chip lh-chip-model" title={`Model: ${m()}${props.work.thinking ? ` · thinking ${props.work.thinking}` : ''}`} data-testid="lh-chip-model">
            {prettyModelId(m())}
            <Show when={props.work.thinking}>
              <span class="lh-chip-think">{props.work.thinking}</span>
            </Show>
          </span>
        )}
      </Show>
    </>
  );
}

/** Aligned numbers: tool calls, elapsed, cost (each only when known). */
export function Metrics(props: { work: LhWork; now: number }) {
  const elapsed = () => workElapsedMs(props.work, props.now);
  return (
    <span class="lh-metrics" data-testid="lh-metrics">
      <Show when={props.work.toolCalls > 0}>
        <span class="lh-metric-item" title="Tool calls">
          <Wrench size={11} /> {props.work.toolCalls}
        </span>
      </Show>
      <Show when={elapsed() !== undefined}>
        <span class="lh-metric-item" title="Time running">
          <Timer size={11} /> {durationText(elapsed()!)}
        </span>
      </Show>
      <Show when={props.work.cost !== undefined}>
        <span class="lh-metric-item" title="Cost">
          <CircleDollarSign size={11} /> {props.work.cost! < 0.01 ? `$${props.work.cost!.toFixed(4)}` : `$${props.work.cost!.toFixed(2)}`}
        </span>
      </Show>
    </span>
  );
}

/** Segmented progress bar: done · running · error · failed; the rest is the track. */
export function LhBar(props: { counts: LhGraphCounts; label: string; total?: number }) {
  const segs = () => progressSegments(props.counts);
  return (
    <div class="lh-bar" role="progressbar" aria-label={props.label} aria-valuemin={0} aria-valuemax={100} data-testid="lh-bar">
      <For each={segs()}>{(s) => <span data-status={s.status} style={{ width: `${s.pct}%` }} data-testid={`lh-bar-${s.status}`} />}</For>
    </div>
  );
}

/** One node row/card: name · status chip · profile/model chips · metrics; instruction below. */
export function NodeRow(props: {
  work: LhWork;
  now: number;
  open: boolean;
  onToggle: () => void;
  onRetry?: (w: LhWork) => void;
  retrying?: boolean;
  onOpenSession?: (w: LhWork) => void;
  selected?: boolean;
  children?: JSX.Element;
}) {
  const w = () => props.work;
  const waiting = () => (w().nextRetryAt && w().nextRetryAt! > props.now ? `retry in ${durationText(w().nextRetryAt! - props.now)}` : undefined);
  return (
    <li class="lh-row" data-status={w().status} data-tone={STATUS_META[w().status].tone} data-open={props.open ? '' : undefined} data-selected={props.selected ? '' : undefined} data-testid="lh-node">
      <div class="lh-row-top">
        <button type="button" class="lh-row-main" aria-expanded={props.open} onClick={props.onToggle} data-testid="lh-node-toggle">
          <span class="lh-row-name" title={w().workId}>{w().workId}</span>
          <StatusChip status={w().status} />
        </button>
        <Show when={w().canRetry && props.onRetry}>
          <button type="button" class="lh-retry" disabled={props.retrying} onClick={() => props.onRetry!(w())} data-testid="lh-node-retry" aria-label={`Retry ${w().workId}`}>
            <Show when={props.retrying} fallback={<RotateCcw size={12} />}>
              <Loader2 size={12} class="lh-spin" />
            </Show>
            Retry
          </button>
        </Show>
        <Show when={props.onOpenSession}>
          <button type="button" class="lh-icon-btn" aria-label={`Open the ${w().workId} worker session`} title="Open session" data-testid="lh-node-open" onClick={() => props.onOpenSession!(w())}>
            <ExternalLink size={13} />
          </button>
        </Show>
      </div>
      <div class="lh-row-chips">
        <RunChips work={w()} />
        <Metrics work={w()} now={props.now} />
        <Show when={waiting()}>{(t) => <span class="lh-metric-item lh-wait">{t()}</span>}</Show>
      </div>
      <Show when={w().instruction}>
        <p class="lh-row-text" data-open={props.open ? '' : undefined} data-testid="lh-node-text">{w().instruction}</p>
      </Show>
      <Show when={w().status === 'errored' && w().error}>
        {(e) => <p class="lh-row-error" data-testid="lh-node-error">{props.open ? e() : e().split('\n')[0]}</p>}
      </Show>
      <Show when={props.open}>{props.children}</Show>
    </li>
  );
}

/** The connected vertical timeline (UniPi's delivery-state style), newest first. */
export function LhTimeline(props: { entries: readonly LhTimelineEntry[]; onPickNode?: (id: string) => void; empty?: string; limit?: number }) {
  const shown = () => props.entries.slice(0, props.limit ?? 80);
  return (
    <Show when={shown().length > 0} fallback={<p class="status-empty">{props.empty ?? 'No events yet.'}</p>}>
      <ol class="lh-tl" aria-label="Events" data-testid="lh-timeline">
        <For each={shown()}>
          {(e) => (
            <li class="lh-tl-item" data-tone={e.tone} data-kind={e.kind} data-testid="lh-tl-item">
              <span class="lh-tl-dot" aria-hidden="true" />
              <div class="lh-tl-body">
                <div class="lh-tl-head">
                  <span class="lh-tl-title">{e.title}</span>
                  <Show when={e.workId}>
                    {(id) => (
                      <button type="button" class="lh-tl-node" onClick={() => props.onPickNode?.(id())} title={`Go to ${id()}`}>
                        {id()}
                      </button>
                    )}
                  </Show>
                  <span class="lh-tl-actor" data-testid="lh-tl-actor">{e.by}</span>
                  <time class="lh-tl-time" title={startedText(e.ts)}>{clockText(e.ts)}</time>
                </div>
                <Show when={e.detail}>{(d) => <p class="lh-tl-detail">{d()}</p>}</Show>
              </div>
            </li>
          )}
        </For>
      </ol>
    </Show>
  );
}

export { eventTitle };
