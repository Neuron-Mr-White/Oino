/**
 * The long-horizon graph / swarm view ([lh-ui]): header (task, state, dates), segmented
 * progress bar, a toolbar (List · Diagram · Timeline, "Show errors", History, Refresh) and the
 * chosen body. Data is the extension's `long-horizon.graph` service (snapshot / history /
 * retry — docs/dev/v2/qa/long-horizon-errored.md), refetched when the progress counts move.
 * History opens an earlier graph read-only in the same views. A host whose extension lacks the
 * service falls back to the old `/visualize-progress` inspector (`LhGraph`).
 */
import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowLeft, History, ListTree, Network, RefreshCw, Rows3 } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { serviceCall } from '../session/modalData';
import { openSubagentTab } from '../panes/tabs/sessionTabs';
import { fetchAgents } from '../panes/tabs/SubagentTab';
import { useClock } from '../../lib/clock';
import { LH_EXTENSION_ID } from '../../lib/status/lh';
import {
  ERROR_STATUSES,
  graphProgress,
  matchesNodeFilter,
  FILTERS,
  type NodeCategory,
  graphTimeline,
  newestFirst,
  parseHistory,
  parseSnapshot,
  refusalText,
  type LhGraphCounts,
  type LhGraphSnap,
  type LhHistoryRow,
  type LhWork,
} from '../../lib/status/lhGraph';
import { LhGraph, workOfAgent } from './LhGraph';
import { LhDiagram } from './LhDiagram';
import { LhBar, LhTimeline, NodeRow, StatusChip } from './LhParts';
import { durationText, startedText } from './lhProgress';
import './lh.css';

type Tab = 'list' | 'diagram' | 'timeline';
const GRAPH_SERVICE = 'long-horizon.graph';

/** Counts with the error statuses zeroed (the "Show errors" off view). Pure. */
export function countsWithout(counts: LhGraphCounts, hide: boolean): LhGraphCounts {
  if (!hide) return counts;
  const out = { ...counts };
  for (const s of ERROR_STATUSES) out[s] = 0;
  return out;
}

const outcome = (g: Pick<LhHistoryRow, 'state' | 'counts' | 'abandoned' | 'finishReason'>): { label: string; tone: 'run' | 'ok' | 'fail' | 'off' | 'error' } => {
  if (g.state === 'open') return { label: 'Open', tone: 'run' };
  if (g.state === 'stopped') return { label: 'Stopped', tone: 'off' };
  if (g.counts.failed > 0) return { label: 'Finished with failures', tone: 'fail' };
  if (g.counts.errored > 0) return { label: 'Finished with errors', tone: 'error' };
  return { label: 'Finished', tone: 'ok' };
};

export function HistoryList(props: { rows: readonly LhHistoryRow[]; loading: boolean; onPick: (id: string) => void }) {
  return (
    <Show when={!props.loading} fallback={<div class="lh-skel" data-testid="lh-history-skel" aria-busy="true"><span /><span /><span /></div>}>
      <Show when={props.rows.length > 0} fallback={<p class="status-empty">No earlier graphs in this project.</p>}>
        <ul class="lh-hist" aria-label="Graph history" data-testid="lh-history-list">
          <For each={props.rows}>
            {(g) => {
              const o = outcome(g);
              return (
                <li>
                  <button type="button" class="lh-hist-row" onClick={() => props.onPick(g.graphId)} data-testid="lh-history-row" data-state={g.state}>
                    <span class="lh-hist-top">
                      <span class="lh-hist-title">{g.task}</span>
                      <span class="lh-chip lh-outcome" data-tone={o.tone}>{o.label}</span>
                    </span>
                    <span class="lh-hist-meta">
                      <span class="lh-chip lh-chip-profile">{g.mode}</span>
                      <span>{startedText(g.createdAt)}</span>
                      <span>{g.workCount} step{g.workCount === 1 ? '' : 's'}</span>
                      <Show when={g.counts.completed > 0}><span class="lh-count" data-status="completed">{g.counts.completed} done</span></Show>
                      <Show when={g.counts.errored > 0}><span class="lh-count" data-status="errored">{g.counts.errored} error</span></Show>
                      <Show when={g.counts.failed > 0}><span class="lh-count" data-status="failed">{g.counts.failed} failed</span></Show>
                      <Show when={g.updatedAt > g.createdAt}><span>{durationText(g.updatedAt - g.createdAt)}</span></Show>
                    </span>
                  </button>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
    </Show>
  );
}

export function LhView(props: { s: SessionHandle; counts: string; onToast?: (text: string) => void }) {
  const [tab, setTab] = createSignal<Tab>('list');
  const [page, setPage] = createSignal<'run' | 'history'>('run');
  const [viewing, setViewing] = createSignal<string | undefined>(undefined);
  const [nonce, setNonce] = createSignal(0);
  const [open, setOpen] = createSignal<string | undefined>(undefined);
  const [retrying, setRetrying] = createSignal<string | undefined>(undefined);
  const [stopping, setStopping] = createSignal<string | undefined>(undefined);
  const [unsupported, setUnsupported] = createSignal(false);
  const readOnly = () => viewing() !== undefined;
  const live = () => !readOnly() && page() === 'run';
  const now = useClock(() => (snap.latest?.work ?? []).some((w) => w.status === 'running' || w.nextRetryAt !== undefined) && live());

  // Debounced trigger: counts moved / refresh pressed / another graph picked.
  const [key, setKey] = createSignal('');
  createEffect(
    on(
      () => `${props.counts}#${nonce()}#${viewing() ?? ''}`,
      (k, prev) => {
        const t = setTimeout(() => setKey(k), prev === undefined || readOnly() ? 0 : 500);
        onCleanup(() => clearTimeout(t));
      },
    ),
  );
  const [snap, { refetch: refetchSnap }] = createResource(
    () => key() || undefined,
    async (): Promise<LhGraphSnap | undefined> => {
      try {
        const reply = await serviceCall<unknown>(props.s, GRAPH_SERVICE, { op: 'snapshot', ...(viewing() ? { graphId: viewing() } : {}) }, LH_EXTENSION_ID);
        if (!reply || typeof reply !== 'object' || !('ok' in reply)) throw new Error('no graph service');
        setUnsupported(false);
        return parseSnapshot(reply);
      } catch {
        setUnsupported(true);
        return undefined;
      }
    },
  );
  const [history] = createResource(
    () => (page() === 'history' ? nonce() + 1 : undefined),
    async (): Promise<LhHistoryRow[]> => {
      try {
        return parseHistory(await serviceCall<unknown>(props.s, GRAPH_SERVICE, { op: 'history' }, LH_EXTENSION_ID));
      } catch {
        return [];
      }
    },
  );
  const [agents] = createResource(
    () => (live() ? props.counts : undefined),
    () => fetchAgents(props.s).catch(() => []),
  );
  const workerOf = (w: LhWork) => (agents.latest ?? []).find((a) => workOfAgent(a.label ?? '') === w.workId);

  const g = () => snap.latest;
  const [filters, setFilters] = createSignal<NodeCategory[]>([]);
  createEffect(on(() => g()?.graphId, (id) => {
    let saved: unknown;
    try { saved = id ? JSON.parse(localStorage.getItem(`lh-filters:${id}`) ?? '[]') : []; } catch { saved = []; }
    setFilters(Array.isArray(saved) ? saved.filter((v): v is NodeCategory => FILTERS.includes(v)) : []);
  }));
  const toggleFilter = (category: NodeCategory) => {
    const next = filters().includes(category) ? filters().filter((v) => v !== category) : [...filters(), category];
    setFilters(next);
    try { if (g()) localStorage.setItem(`lh-filters:${g()!.graphId}`, JSON.stringify(next)); } catch { /* private mode */ }
  };
  const clearFilters = () => {
    setFilters([]);
    try { if (g()) localStorage.removeItem(`lh-filters:${g()!.graphId}`); } catch { /* private mode */ }
  };
  const visible = createMemo(() => (g()?.work ?? []).filter((w) => matchesNodeFilter(w, filters())));
  const ordered = createMemo(() => newestFirst(visible()));
  const counts = createMemo(() => g()?.counts ?? ({} as LhGraphCounts));
  const progress = createMemo(() => g() ? graphProgress(g()!) : undefined);
  const hiddenCount = () => (g()?.work.length ?? 0) - visible().length;
  const timeline = createMemo(() => (g() ? graphTimeline({ ...g()!, work: visible() }) : []));

  const retry = async (w: LhWork) => {
    if (retrying()) return;
    setRetrying(w.workId);
    try {
      const reply = await serviceCall<unknown>(props.s, GRAPH_SERVICE, { op: 'retry', workId: w.workId, ...(g() ? { graphId: g()!.graphId } : {}) }, LH_EXTENSION_ID);
      const refused = refusalText(reply);
      if (refused) props.onToast?.(refused);
      else props.onToast?.(`Retrying ${w.workId}`);
    } catch {
      // An older extension build without the service: the slash command does the same.
      void props.s.cmd({ type: 'prompt', message: `/${g()?.mode ?? 'graph'} retry ${w.workId}` }).then(() => props.onToast?.(`Retrying ${w.workId}`)).catch(() => props.onToast?.('Could not retry'));
    } finally {
      setRetrying(undefined);
      setNonce((n) => n + 1);
    }
  };
  const stop = async (w: LhWork) => {
    if (stopping()) return;
    setStopping(w.workId);
    try {
      const reply = await serviceCall<unknown>(props.s, GRAPH_SERVICE, { op: 'stop_operator', workId: w.workId }, LH_EXTENSION_ID);
      const refused = refusalText(reply);
      props.onToast?.(refused ?? `Stopping the ${w.workId} worker`);
    } catch {
      props.onToast?.('Could not stop the worker');
    } finally {
      setStopping(undefined);
      setNonce((n) => n + 1);
    }
  };
  const openSession = (w: LhWork) => {
    const a = workerOf(w);
    if (a) openSubagentTab({ hostId: props.s.hostId, sessionId: props.s.sessionId() }, { agentId: a.id, name: a.label });
  };
  const pickNode = (id: string) => {
    setOpen(id);
    if (tab() === 'timeline') setTab('list');
  };
  const pickHistory = (id: string) => {
    setViewing(id);
    setPage('run');
    setOpen(undefined);
  };
  const backToHistory = () => {
    setViewing(undefined);
    setPage('history');
  };
  const detail = (w: LhWork) => (
    <div class="lh-detail" data-testid="lh-node-detail">
      <Show when={w.error && w.status !== 'errored'}>{(_e) => <p class="lh-row-error">{w.error}</p>}</Show>
      <Show when={w.stopReason}>{(r) => <p class="lh-detail-line">Stopped: {r()}</p>}</Show>
      <Show when={w.blockedBy}>{(r) => <p class="lh-detail-line">Blocked by {r()}</p>}</Show>
      <Show when={w.inputIds.length}><p class="lh-detail-line">After {w.inputIds.map((i) => i.replace(/^r-/, '')).join(', ')}</p></Show>
      <Show when={w.continues + w.autoContinues > 0}><p class="lh-detail-line">Continued {w.continues + w.autoContinues}× ({w.autoContinues} automatic)</p></Show>
      <LhTimeline entries={graphTimeline({ events: [], work: [w] })} limit={12} empty="No events." />
    </div>
  );

  return (
    <Show
      when={!unsupported()}
      fallback={<LhGraph s={props.s} counts={props.counts} onToast={props.onToast} />}
    >
      <div class="lh-view" data-testid="lh-view" data-readonly={readOnly() ? '' : undefined}>
        <Show when={page() === 'history'}>
          <div class="lh-subhead">
            <button type="button" class="lh-back" onClick={() => setPage('run')} data-testid="lh-history-back"><ArrowLeft size={13} /> Back</button>
            <span class="lh-sub-title">History</span>
            <span class="flex-1" />
            <button type="button" class="lh-icon-btn" aria-label="Refresh" title="Refresh" onClick={() => setNonce((n) => n + 1)}><RefreshCw size={13} /></button>
          </div>
          <HistoryList rows={history.latest ?? []} loading={history.loading && !history.latest} onPick={pickHistory} />
        </Show>

        <Show when={page() === 'run'}>
          <Show when={readOnly()}>
            <div class="lh-subhead">
              <button type="button" class="lh-back" onClick={backToHistory} data-testid="lh-snapshot-back"><ArrowLeft size={13} /> History</button>
              <span class="lh-chip lh-outcome" data-tone="off">Read-only snapshot</span>
            </div>
          </Show>
          <Show when={g()} fallback={<Show when={snap.loading} fallback={<p class="status-empty">No graph in this project yet.</p>}><div class="lh-skel" aria-busy="true"><span /><span /><span /></div></Show>}>
            {(gr) => (
              <>
                <header class="lh-head" data-testid="lh-head">
                  <div class="lh-head-top">
                    <span class="lh-head-title" title={gr().task}>{gr().task}</span>
                    <span class="lh-chip lh-outcome" data-tone={outcome(gr()).tone} data-testid="lh-graph-state">{outcome(gr()).label}</span>
                  </div>
                  <div class="lh-head-meta">
                    <span class="lh-chip lh-chip-profile">{gr().mode}</span>
                    <span>{startedText(gr().createdAt)}</span>
                    <Show when={gr().updatedAt > gr().createdAt}><span>{durationText(gr().updatedAt - gr().createdAt)}</span></Show>
                    <Show when={gr().finishReason ?? gr().abandoned}>{(r) => <span class="lh-head-reason" title={r()}>{r()}</span>}</Show>
                  </div>
                  <div class="lh-head-count" data-testid="lh-counts">
                    <strong>{progress()?.finished ? 'Finished' : `${progress()?.done}/${progress()?.denom}`}</strong>
                    <For each={['done', 'stopped', 'replaced', 'blocked', 'errored', 'failed', 'active'] as const}>
                      {(key) => <Show when={key === 'done' || (progress()?.[key] ?? 0) > 0}><span class="lh-count" data-status={key}>{progress()?.[key]} {key === 'errored' ? 'error' : key === 'active' ? 'still running' : key}</span></Show>}
                    </For>
                  </div>
                  <LhBar counts={counts()} label="Graph progress" />
                </header>

                <div class="lh-toolbar" role="toolbar" aria-label="Graph view">
                  <div class="lh-seg" role="tablist">
                    <button type="button" role="tab" aria-selected={tab() === 'list'} class="lh-seg-btn" onClick={() => setTab('list')} data-testid="lh-tab-list"><Rows3 size={12} /> List</button>
                    <button type="button" role="tab" aria-selected={tab() === 'diagram'} class="lh-seg-btn" onClick={() => setTab('diagram')} data-testid="lh-tab-diagram"><Network size={12} /> Diagram</button>
                    <button type="button" role="tab" aria-selected={tab() === 'timeline'} class="lh-seg-btn" onClick={() => setTab('timeline')} data-testid="lh-tab-timeline"><ListTree size={12} /> Timeline</button>
                  </div>
                  <span class="flex-1" />
                  <For each={FILTERS}>{(category) => <button type="button" class="lh-toggle" aria-pressed={filters().includes(category)} data-testid={`lh-filter-${category}`} onClick={() => toggleFilter(category)}>{category[0]!.toUpperCase() + category.slice(1)}</button>}</For>
                  <Show when={filters().length}><button type="button" class="lh-toggle" onClick={clearFilters}>Clear all</button></Show>
                  <Show when={live()}>
                    <button type="button" class="lh-icon-btn lh-refresh" aria-label="Refresh the graph" title="Refresh" onClick={() => (setNonce((n) => n + 1), void refetchSnap())}><RefreshCw size={13} /></button>
                  </Show>
                  <button type="button" class="lh-icon-btn" aria-label="History" title="History — earlier graphs" onClick={() => (setPage('history'), setNonce((n) => n + 1))} data-testid="lh-history"><History size={14} /></button>
                </div>
                <Show when={hiddenCount() > 0}>
                  <p class="lh-hidden-note" data-testid="lh-hidden-note">{hiddenCount()} step{hiddenCount() === 1 ? '' : 's'} hidden</p>
                </Show>

                <Show when={tab() === 'list'}>
                  <Show when={ordered().length > 0} fallback={<p class="status-empty">No steps to show.</p>}>
                    <ul class="lh-rows" aria-label="Graph nodes" data-testid="lh-nodes">
                      <For each={ordered()}>
                        {(w) => (
                          <NodeRow
                            work={w}
                            now={now()}
                            open={open() === w.workId}
                            onToggle={() => setOpen((c) => (c === w.workId ? undefined : w.workId))}
                            onRetry={readOnly() ? undefined : retry}
                            retrying={retrying() === w.workId}
                            onStop={readOnly() ? undefined : stop}
                            stopping={stopping() === w.workId}
                            onOpenSession={live() && workerOf(w) ? openSession : undefined}
                          >
                            {detail(w)}
                          </NodeRow>
                        )}
                      </For>
                    </ul>
                    <Show when={gr().omittedWork > 0}><p class="lh-hidden-note">{gr().omittedWork} older settled steps not shown</p></Show>
                  </Show>
                </Show>

                <Show when={tab() === 'diagram'}>
                  <LhDiagram work={visible()} now={now()} selected={open()} onSelect={pickNode} perRow={2} height={440} />
                  <Show when={visible().find((w) => w.workId === open())}>
                    {(w) => (
                      <ul class="lh-rows lh-rows-detail">
                        <NodeRow work={w()} now={now()} open onToggle={() => setOpen(undefined)} onRetry={readOnly() ? undefined : retry} retrying={retrying() === w().workId} onStop={readOnly() ? undefined : stop} stopping={stopping() === w().workId} onOpenSession={live() && workerOf(w()) ? openSession : undefined} selected>
                          {detail(w())}
                        </NodeRow>
                      </ul>
                    )}
                  </Show>
                </Show>

                <Show when={tab() === 'timeline'}>
                  <LhTimeline entries={timeline()} onPickNode={pickNode} />
                  <Show when={gr().eventsDropped > 0}><p class="lh-hidden-note">{gr().eventsDropped} older events dropped</p></Show>
                </Show>
              </>
            )}
          </Show>
        </Show>
      </div>
    </Show>
  );
}

export { StatusChip };
