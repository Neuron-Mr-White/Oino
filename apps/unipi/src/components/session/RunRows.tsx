/**
 * The ONE row + filter-chip set of the Running view ([owner-fix-3/running-pane]): Subagents and
 * Background tasks are lists of the same shape — status icon, name + chip(s), a muted second
 * line, elapsed, Stop, open-in-pane — so both sections render through {@link WorkRow} and
 * {@link FilterChips} (the same `.sm-run*` look). Pure view: the sections own data and actions.
 */
import { For, Show, type JSX } from 'solid-js';
import { Ban, Check, LoaderCircle, Moon, PanelRightOpen, Square, X } from 'lucide-solid';
import { ROW_STATUS_LABEL, RUN_FILTERS, type RunFilter, type RunRowStatus } from '../../lib/status/subagentRows';

/** The run's status icon — the timeline AgentCard's set (spinner / check / x / ban; idle = moon). */
export function RunStatusIcon(props: { status: RunRowStatus }) {
  return (
    <span class="sm-run-icon" data-status={props.status} title={ROW_STATUS_LABEL[props.status]} aria-label={ROW_STATUS_LABEL[props.status]} role="img">
      <Show when={props.status === 'running'}>
        <LoaderCircle size={15} class="sm-spin" />
      </Show>
      <Show when={props.status === 'done'}>
        <Check size={15} />
      </Show>
      <Show when={props.status === 'failed'}>
        <X size={15} />
      </Show>
      <Show when={props.status === 'cancelled'}>
        <Ban size={15} />
      </Show>
      <Show when={props.status === 'idle'}>
        <Moon size={15} />
      </Show>
    </span>
  );
}

export type RowChip = { text: string; title?: string; key?: string };
export type RowStat = { key: string; text: string; title?: string; tone?: 'error' };

/** One run / task row. A click on the row (or Enter) opens it; the buttons keep their own click. */
export function WorkRow(props: {
  status: RunRowStatus;
  title: string;
  chips?: RowChip[];
  /** A trailing dim label on the first line (the subagent's model). */
  model?: { text: string; title?: string };
  /** The muted second line, as dot-separated stats. */
  sub?: RowStat[];
  /** …or one text (the task's command, mono). */
  subText?: string;
  /** Right column: elapsed, then these dim stats. */
  elapsed?: { text: string; title?: string };
  trailing?: string;
  /** Extra buttons before Stop. */
  actions?: JSX.Element;
  stop?: { onStop: () => void; title: string };
  open: { onOpen: () => void; label: string };
  testId: string;
  dataRun: string;
}) {
  const press = (fn: () => void) => (e: MouseEvent) => {
    e.stopPropagation();
    fn();
  };
  return (
    <div
      class="sm-run"
      data-status={props.status}
      data-testid={props.testId}
      data-run={props.dataRun}
      role="listitem"
      tabindex={0}
      title={props.open.label}
      onClick={() => props.open.onOpen()}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault();
          props.open.onOpen();
        }
      }}
    >
      <RunStatusIcon status={props.status} />
      <div class="sm-run-main">
        <div class="sm-run-line">
          <span class="sm-run-title" title={props.title}>{props.title}</span>
          <For each={props.chips ?? []}>{(c) => <span class="sm-run-chip" data-chip={c.key} title={c.title ?? c.text}>{c.text}</span>}</For>
          <Show when={props.model}>{(m) => <span class="sm-run-model" title={m().title}>{m().text}</span>}</Show>
        </div>
        <div class="sm-run-sub">
          <Show when={props.subText}>{(t) => <span class="sm-run-cmd" title={t()}>{t()}</span>}</Show>
          <For each={props.sub ?? []}>{(p) => <span class="sm-run-stat" classList={{ 'sm-run-err': p.tone === 'error' }} data-stat={p.key} title={p.title}>{p.text}</span>}</For>
        </div>
      </div>
      <Show when={props.elapsed || props.trailing}>
        <span class="sm-run-time">
          <Show when={props.elapsed}>{(e) => <span class="sm-elapsed" title={e().title}>{e().text}</span>}</Show>
          <Show when={props.trailing}>{(t) => <span class="sm-run-trail">{t()}</span>}</Show>
        </span>
      </Show>
      <span class="sm-row-actions sm-run-actions" onClick={(e) => e.stopPropagation()}>
        {props.actions}
        <Show when={props.stop}>
          {(st) => (
            <button type="button" data-tone="danger" onClick={press(st().onStop)} aria-label="Stop" title={st().title}>
              <Square size={13} />
              <span class="sm-btn-label">Stop</span>
            </button>
          )}
        </Show>
        <button type="button" class="sm-icon-btn" onClick={press(props.open.onOpen)} aria-label={props.open.label} title={props.open.label}>
          <PanelRightOpen size={15} />
        </button>
      </span>
    </div>
  );
}

/** All / Running / Done / Failed (Cancelled only when there are some) — one filter bar for every list. */
export function FilterChips(props: { counts: Record<RunFilter, number>; filter: RunFilter; onFilter: (f: RunFilter) => void; label: string; testPrefix?: string }) {
  const chips = () => RUN_FILTERS.filter((f) => f.id !== 'cancelled' || props.counts.cancelled > 0 || props.filter === 'cancelled');
  return (
    <div class="sm-chips" role="toolbar" aria-label={props.label}>
      <For each={chips()}>
        {(f) => (
          <button type="button" class="sm-chip" aria-pressed={props.filter === f.id} data-filter={f.id} data-testid={`${props.testPrefix ?? 'sm-filter'}-${f.id}`} onClick={() => props.onFilter(props.filter === f.id && f.id !== 'all' ? 'all' : f.id)}>
            {f.label}
            <span class="sm-chip-n">{props.counts[f.id]}</span>
          </button>
        )}
      </For>
    </div>
  );
}
