/**
 * The opt-in subagents tree under a session row ([ui-sessions]): a chat's
 * subagent runs (title, profile, status icon, duration) with a status filter
 * in the tree header. Off by default — Properties › "Show subagents" turns it
 * on for every chat whose runs the app knows (an open chat), the row menu's
 * "Show subagents" for one chat (that one may open the chat's stream to load
 * them, the same as clicking the row would).
 *
 * Data: the subagents extension's sticky `unipi:subagents:list` topic on the
 * chat's stream (`{records:[{id,title,profile,status,startedAt,endedAt}]}` —
 * the data the Running tab / work tray show), else `_oino/agents` (the
 * Session modal's Running tab). Read runs are cached per chat so an LRU
 * eviction of the stream does not empty the tree. Clicking a run opens its
 * transcript (`openSubagentTab`, the step rows' / completion cards' surface).
 */
import { createEffect, createSignal, For, on, onCleanup, Show, type JSX } from 'solid-js';
import { CircleCheck, CircleSlash, CircleX, LoaderCircle, Bot, Moon } from 'lucide-solid';
import { appNavigate } from '../../lib/nav';
import { useClock } from '../../lib/clock';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { SUBAGENTS_LIST_TOPIC } from '../../lib/status/running';
import { scopedSubagents } from '../../lib/status/scopedSubagents';
import type { RowVariant } from './SessionRow';

export type SubRun = { id: string; title: string; profile?: string; status: string; startedAt?: number; endedAt?: number };

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

/** `{records:[…]}` (the topic) → runs; `undefined` when the value is not a list. */
export function runsFromTopic(data: unknown, sessionId?: string): SubRun[] | undefined {
  const records = scopedSubagents(data, sessionId);
  if (!records) return undefined;
  const out: SubRun[] = [];
  for (const raw of records) {
    const r = raw as Record<string, unknown>;
    const id = str(r?.id);
    if (!id) continue;
    out.push({
      id,
      title: str(r.title) ?? str(r.task)?.slice(0, 80) ?? id,
      ...(str(r.profile) ? { profile: str(r.profile) } : {}),
      status: str(r.status) ?? 'running',
      ...(num(r.startedAt) ? { startedAt: num(r.startedAt) } : {}),
      ...(num(r.endedAt) ? { endedAt: num(r.endedAt) } : {}),
    });
  }
  return out.sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0));
}

/** `_oino/agents` (`AgentInfo` tree) → runs (the nested agents only). */
export function runsFromAgents(agents: unknown): SubRun[] {
  if (!Array.isArray(agents)) return [];
  return agents
    .filter((a) => a && typeof a.id === 'string' && (a.depth ?? 1) > 0)
    .map((a) => ({
      id: a.id as string,
      title: str(a.label) ?? (a.id as string),
      ...(str(a.role) ? { profile: str(a.role) } : {}),
      status: a.state === 'running' ? 'running' : a.state === 'idle' ? 'idle' : 'completed',
    }));
}

const key = (hostId: string, sessionId: string) => `${hostId}\u0000${sessionId}`;
/** Runs read per chat (kept past an LRU eviction of the chat's stream). */
const [cache, setCache] = createSignal<Record<string, SubRun[]>>({});
/** The tree header's status filter per chat (`undefined` = the default, "In progress"). */
const [filters, setFilters] = createSignal<Record<string, string | undefined>>({});
const asked = new Set<string>();

/**
 * The status the tree shows: the user's pick, else "In progress" (owner
 * 2026-11-06). A chat without in-progress runs keeps that default — the tree
 * shows its empty state, it never flips to "All" on its own.
 */
export const DEFAULT_FILTER = 'running';
/** The chat's effective filter (`undefined` = no filter row, everything shows). */
export function filterOf(hostId: string, sessionId: string, runs: readonly unknown[] | undefined): string | undefined {
  if (!hasFilter(runs)) return undefined;
  return filters()[key(hostId, sessionId)] ?? DEFAULT_FILTER;
}

/** Known runs of a chat (reactive): the live topic, else the cache. */
export function knownRuns(hostId: string, sessionId: string): SubRun[] | undefined {
  const h = sessions().peek(hostId, sessionId);
  const live = h ? runsFromTopic(h.state.topics[SUBAGENTS_LIST_TOPIC]?.data, sessionId) : undefined;
  return live ?? cache()[key(hostId, sessionId)];
}

/** Starts watching an open chat's subagents topic (cheap; the Capsule subscribes it too). */
export function watchRuns(h: SessionHandle | undefined): void {
  h?.subscribeTopics([SUBAGENTS_LIST_TOPIC]);
}

export const STATUS_WORDS: Record<string, string> = { running: 'In progress', idle: 'Idle', completed: 'Completed', failed: 'Failed', cancelled: 'Cancelled' };
const STATUS_ORDER = ['running', 'idle', 'completed', 'failed', 'cancelled'];
const statusWord = (s: string) => STATUS_WORDS[s] ?? s.charAt(0).toUpperCase() + s.slice(1);

/** Row geometry (px): the list windows tall rows with these. */
export function treeMetrics(variant: RowVariant): { head: number; row: number } {
  return variant === 'page' ? { head: 34, row: 40 } : { head: 26, row: 26 };
}

/** [sidebar-ui] A tree of this many runs or fewer needs no status filter. */
export const FILTER_MIN = 3;
/** Whether the tree shows its filter row (more than {@link FILTER_MIN} runs). */
export const hasFilter = (runs: readonly unknown[] | undefined) => (runs?.length ?? 0) > FILTER_MIN;

export function filteredRuns(hostId: string, sessionId: string, runs: readonly SubRun[]): SubRun[] {
  const f = filterOf(hostId, sessionId, runs);
  return f && f !== 'all' ? runs.filter((r) => r.status === f) : [...runs];
}

/** perf-app: rows the tree shows before its runs scroll in a bounded, windowed box (a 200-child graph). */
export const TREE_ROWS_MAX = 12;
/** Rows mounted beyond the visible window, each side. */
const TREE_OVERSCAN = 4;

/** The tree's height for `runs` (`undefined` = not loaded: one placeholder row). */
export function treeHeight(variant: RowVariant, hostId: string, sessionId: string, runs: readonly SubRun[] | undefined): number {
  const m = treeMetrics(variant);
  if (!runs || runs.length === 0) return m.row + 4;
  return (hasFilter(runs) ? m.head : 0) + Math.min(TREE_ROWS_MAX, Math.max(1, filteredRuns(hostId, sessionId, runs).length)) * m.row + 4;
}

/** "42s", "3m 04s", "1h 02m". */
export function formatDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
}

function StatusIcon(props: { status: string }): JSX.Element {
  return (
    <span class="sa-icon" data-status={props.status} title={statusWord(props.status)} aria-label={statusWord(props.status)} role="img">
      <Show when={props.status === 'running'}>
        <LoaderCircle size={12} class="srow-spin" />
      </Show>
      <Show when={props.status === 'completed'}>
        <CircleCheck size={12} />
      </Show>
      <Show when={props.status === 'failed'}>
        <CircleX size={12} />
      </Show>
      <Show when={props.status === 'cancelled'}>
        <CircleSlash size={12} />
      </Show>
      <Show when={props.status === 'idle'}>
        <Moon size={12} />
      </Show>
      <Show when={!['running', 'completed', 'failed', 'cancelled', 'idle'].includes(props.status)}>
        <Bot size={12} />
      </Show>
    </span>
  );
}

export function SubagentTree(props: { hostId: string; sessionId: string; cwd?: string; variant: RowVariant; /** Turned on for this chat (may load). */ explicit: boolean }): JSX.Element {
  const k = () => key(props.hostId, props.sessionId);
  const handle = () => sessions().peek(props.hostId, props.sessionId);
  const runs = () => knownRuns(props.hostId, props.sessionId);
  const [loading, setLoading] = createSignal(false);

  // Keep the cache fresh while the chat is open; fall back to `_oino/agents` once when the
  // topic never arrives (an older extension / runtime).
  createEffect(
    on(handle, (h) => {
      if (!h) return;
      watchRuns(h);
    }),
  );
  createEffect(() => {
    const h = handle();
    if (!h) return;
    const live = runsFromTopic(h.state.topics[SUBAGENTS_LIST_TOPIC]?.data, props.sessionId);
    if (live) {
      setCache((c) => ({ ...c, [k()]: live }));
      setLoading(false);
      return;
    }
    if (h.state.conn === 'live' && !asked.has(k())) {
      asked.add(k());
      void h
        .cmd<{ agents?: unknown }>({ type: '_oino/agents' })
        .then((reply) => {
          if (!runsFromTopic(h.state.topics[SUBAGENTS_LIST_TOPIC]?.data, props.sessionId)) setCache((c) => ({ ...c, [k()]: runsFromAgents(reply?.agents) }));
        })
        .catch(() => setCache((c) => ({ ...c, [k()]: c[k()] ?? [] })))
        .finally(() => setLoading(false));
    }
  });

  const load = () => {
    setLoading(true);
    asked.delete(k());
    watchRuns(sessions().ensure(props.hostId, props.sessionId));
  };
  const statuses = () => {
    const present = new Set((runs() ?? []).map((r) => r.status));
    return [...STATUS_ORDER.filter((s) => present.has(s)), ...[...present].filter((s) => !STATUS_ORDER.includes(s))];
  };
  const count = (s: string) => (runs() ?? []).filter((r) => r.status === s).length;
  /** The filter the tree shows now: the user's pick, else "In progress". */
  const active = () => filterOf(props.hostId, props.sessionId, runs());
  const setFilter = (s: string | undefined) => setFilters((cur) => ({ ...cur, [k()]: s }));
  const shown = () => filteredRuns(props.hostId, props.sessionId, runs() ?? []);
  const open = (run: SubRun) => {
    appNavigate(`/s/${encodeURIComponent(props.hostId)}/${encodeURIComponent(props.sessionId)}`);
    // The side pane follows the shown chat's workspace: open the tab once the route moved.
    // (Loaded on demand: the pane modules stay out of the sidebar's import graph.)
    const base = { hostId: props.hostId, sessionId: props.sessionId, ...(props.cwd ? { cwd: props.cwd } : {}) };
    setTimeout(() => void import('../panes/tabs/sessionTabs').then((m) => m.openSubagentTab(base, { agentId: run.id, name: run.title })), 60);
  };
  const m = () => treeMetrics(props.variant);
  // A running run's duration ticks (once a second, only while one runs).
  const now = useClock(() => (runs() ?? []).some((r) => r.status === 'running' && !r.endedAt));

  return (
    <div class="sa-tree" data-variant={props.variant} role="group" aria-label="Subagents" data-testid={`sa-tree-${props.sessionId}`}>
      <Show
        when={runs()?.length}
        fallback={
          <div class="sa-row sa-empty" style={{ height: `${m().row}px` }}>
            <Show when={runs() === undefined} fallback={<span>No subagent runs</span>}>
              <Show when={!loading()} fallback={<span>Loading subagents…</span>}>
                <button type="button" class="sa-load" data-testid={`sa-load-${props.sessionId}`} onClick={load}>
                  Load subagents
                </button>
              </Show>
            </Show>
          </div>
        }
      >
        <Show when={runs()!.length > FILTER_MIN}>
        <div class="sa-head" style={{ height: `${m().head}px` }} role="toolbar" aria-label="Filter subagents by status">
          <button type="button" class="sa-filter" data-on={active() === 'all' ? '' : undefined} onClick={() => setFilter('all')}>
            All <span class="sa-n">{runs()!.length}</span>
          </button>
          <For each={statuses()}>
            {(s) => (
              <button type="button" class="sa-filter" data-on={active() === s ? '' : undefined} data-status={s} title={statusWord(s)} data-testid={`sa-filter-${s}`} onClick={() => setFilter(active() === s ? 'all' : s)}>
                {statusWord(s)} <span class="sa-n">{count(s)}</span>
              </button>
            )}
          </For>
        </div>
        </Show>
        <Show when={shown().length} fallback={<div class="sa-row sa-empty" style={{ height: `${m().row}px` }}>None {statusWord(active() ?? '').toLowerCase()}</div>}>
          <TreeRows runs={shown()} row={m().row}>
            {(run) => (
              <button
                type="button"
                class="sa-row"
                style={{ height: `${m().row}px` }}
                title={`${run.title}${run.profile ? ` · ${run.profile}` : ''} · ${statusWord(run.status)}`}
                data-testid={`sa-run-${run.id}`}
                onClick={() => open(run)}
              >
                <StatusIcon status={run.status} />
                <span class="sa-title">{run.title}</span>
                <Show when={run.profile}>
                  <span class="sa-profile">{run.profile}</span>
                </Show>
                <Show when={run.startedAt}>
                  <span class="sa-dur">{formatDuration((run.endedAt ?? now()) - run.startedAt!)}</span>
                </Show>
              </button>
            )}
          </TreeRows>
        </Show>
      </Show>
    </div>
  );
}

/**
 * The run rows: all of them up to `TREE_ROWS_MAX`, else a box of that many
 * rows that scrolls, mounting only the rows in view (+ overscan). Rows are
 * keyed by run id, so a topic update does not remount the window.
 */
function TreeRows(props: { runs: SubRun[]; row: number; children: (run: SubRun) => JSX.Element }): JSX.Element {
  const byId = () => new Map(props.runs.map((r) => [r.id, r] as const));
  const [top, setTop] = createSignal(0);
  const windowed = () => props.runs.length > TREE_ROWS_MAX;
  const ids = () => {
    if (!windowed()) return props.runs.map((r) => r.id);
    const from = Math.max(0, Math.floor(top() / props.row) - TREE_OVERSCAN);
    const to = Math.min(props.runs.length, from + TREE_ROWS_MAX + 2 * TREE_OVERSCAN);
    return props.runs.slice(from, to).map((r) => r.id);
  };
  const indexOf = () => new Map(props.runs.map((r, i) => [r.id, i] as const));
  const rows = (
    <For each={ids()}>
      {(id) => (
        <Show when={byId().get(id)}>
          {(run) => (
            <Show when={windowed()} fallback={props.children(run())}>
              <div class="sa-vrow" style={{ transform: `translateY(${(indexOf().get(id) ?? 0) * props.row}px)` }}>
                {props.children(run())}
              </div>
            </Show>
          )}
        </Show>
      )}
    </For>
  );
  return (
    <Show when={windowed()} fallback={rows}>
      <div class="sa-scroll" style={{ height: `${TREE_ROWS_MAX * props.row}px` }} data-testid="sa-scroll" onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
        <div class="sa-space" style={{ height: `${props.runs.length * props.row}px` }}>
          {rows}
        </div>
      </div>
    </Show>
  );
}
