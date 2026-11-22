/**
 * Running tab (WP-A17, Z-95; [running-tab] 2026-11-03): **the** full Running view.
 *
 * - **Subagents** — ONE list of the chat's runs: the subagent run records
 *   (`unipi:subagents:list` topic, `subagents.list` service — structured) merged
 *   with the host's nested-agent tree (`_oino/agents`), one row per run
 *   (`lib/status/subagentRows`). Status icon, title, profile chip, model, duration,
 *   a muted line with tools / tokens / cost; Stop and To background on live runs,
 *   Open transcript on every run; All / Running / Done / Failed chips; newest
 *   first; windowed past {@link RUNS_VIRTUAL_AT} rows. Before, the same runs showed
 *   twice: as the agent tree and as the subagents extension's TUI tray pane.
 * - **Bash** — foreground bash calls (`_oino/running_bash`) with Stop run and
 *   "Send to background" (the watchdog's `/bg-detach`, adopted as a background task).
 * - **Sidekick** — the fusion sidekick (`fusion.status`) while a handoff is live.
 * - **Work tray** — the OTHER producers' `work-tray.tab` registrations (background
 *   tasks, dream, workflow, …) as their OUS trees through U9 `NodeView`, actions
 *   routed to the tab's owner through `work-tray.action`. The subagents tab is
 *   the terminal's (`surfaces: ["tui"]`) and is not asked for at all.
 *
 * Empty sections are not drawn; when nothing is there at all the tab is one
 * quiet "Nothing running" line.
 */
import { useClock } from '../../lib/clock';
import { createEffect, createMemo, createResource, createSignal, For, Show, type Accessor, type JSX } from 'solid-js';
import { ArrowDownToLine, Bot, CircleCheck, Layers, LoaderCircle, RefreshCw, Sparkles, SquareTerminal, Terminal } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { EntryContext, type EntryActions, type UiTarget } from '../../ui/actions';
import { NodeView } from '../../ui/NodeView';
import { useVirtual, VirtualWindow } from '../../ui/virtual';
import { formatDurationMs, liveSource, parseAgents, parseExtensions, parseRunningBash, parseSubagentRuns, parseTrayTabs, serviceCall, type AgentInfo, type SubagentRun, type TrayTab } from './modalData';
import { scopedSubagents } from '../../lib/status/scopedSubagents';
import { bgTasksTopics, bgTopicData, runningCounts, runningTotal, runRowsOf, SUBAGENTS_LIST_TOPIC, visibleTrayTabsOf, type RunningData } from '../../lib/status/running';
import { filterCounts, isLive, matchesFilter, profileChip, ROW_STATUS_LABEL, RUN_FILTERS, rowDurationMs, shortModel, statParts, SUBAGENTS_EXT, type RunFilter, type RunRow, type RunRowStatus } from '../../lib/status/subagentRows';
import { openBgTaskTab, openSubagentTab } from '../panes/tabs/sessionTabs';
import { FilterChips, RunStatusIcon, WorkRow, type RowStat } from './RunRows';
import { BG_EXT, BG_TAB, bgExitText, formatBytes, parseBgTasks, type BgRow } from '../../lib/status/bgTasks';
import { setSessionModalTarget } from './modalTarget';

export { agentSeenAt } from '../../lib/status/running';


/** Stops a bash tool call: the run's Stop (pi `abort`) — the run is what owns the call. */
export const stopRun = (s: SessionHandle) => s.cmd({ type: 'abort' }).then(() => undefined);

/**
 * Moves the running bash call into a background task: the watchdog's
 * `/bg-detach` command (it adopts the call through `unipi.background-tasks`).
 * Core dispatches `/<command>` prompts as extension commands, not model turns.
 * The command detaches *the* running call, so it is offered for a single one.
 */
export const detachBash = (s: SessionHandle) =>
  s.cmd({ type: 'prompt', message: '/bg-detach' }).then(() => undefined);

/** The detach action is only truthful when one call runs and the command is there. */
export function canDetachBash(s: SessionHandle, calls: number): boolean {
  return calls === 1 && s.state.commands.some((c) => c.name === 'bg-detach');
}

/**
 * One fetch pass: the native sections' data, the subagent run records and every
 * other loaded extension's work-tray tabs. `work-tray.tab` is a fan-out service;
 * the frontend's `_oino/service_call` reaches one server, so the app asks each
 * loaded extension (unknown services answer `E_NOT_FOUND` and are dropped). The
 * subagents extension is asked for its records (`subagents.list`) instead of its
 * tray pane — the pane is TUI text and building it reads every child transcript.
 */
export async function fetchRunning(s: SessionHandle): Promise<{ bash: ReturnType<typeof parseRunningBash>; agents: AgentInfo[]; runs?: SubagentRun[]; bg?: BgRow[]; sidekick?: { label: string; detail: string }; tabs: TrayTab[] }> {
  const [bash, agents, extensions] = await Promise.all([
    s.cmd<unknown>({ type: '_oino/running_bash' }).catch(() => undefined),
    s.cmd<unknown>({ type: '_oino/agents' }).catch(() => undefined),
    s.cmd<unknown>({ type: '_oino/extensions' }).catch(() => undefined),
  ]);
  const loaded = parseExtensions(extensions).filter((e) => e.loaded);
  const tabs: TrayTab[] = [];
  let runs: SubagentRun[] | undefined;
  if (loaded.length > 0) {
    const replies = await Promise.all(
      loaded.map((e) =>
        e.id === SUBAGENTS_EXT
          ? serviceCall<unknown>(s, 'subagents.list', undefined, e.id)
              .then((v) => {
                runs = parseSubagentRuns(scopedSubagents(v, s.sessionId()));
                return [] as TrayTab[];
              })
              .catch(() => [] as TrayTab[])
          : serviceCall<unknown>(s, 'work-tray.tab', { open: true }, e.id)
              // The tray fills `owner` for the TUI; here the app asked this extension, so it owns the tab.
              .then((v) => parseTrayTabs(v).map((t) => (t.owner ? t : { ...t, owner: e.id })))
              .catch(() => [] as TrayTab[]),
      ),
    );
    for (const list of replies) tabs.push(...list);
  }
  tabs.sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
  let sidekick: { label: string; detail: string } | undefined;
  if (loaded.some((e) => e.id === 'unipi.fusion')) {
    const status = await serviceCall<{ leadName?: string; sidekickName?: string; busy?: boolean }>(s, 'fusion.status', undefined, 'unipi.fusion').catch(() => undefined);
    if (status?.busy) {
      sidekick = {
        label: `Fusion · ${status.leadName || 'lead'} ◆ ${status.sidekickName || 'sidekick'}`,
        detail: 'Handoff running',
      };
    }
  }
  return { bash: parseRunningBash(bash), agents: parseAgents(agents), ...(runs ? { runs } : {}), sidekick, tabs };
}

/** Routes one tray-tree action (`<tab>:<verb>[:<id>]`, value) to its owner. */
export function trayAction(s: SessionHandle, tab: TrayTab, actionId: string | undefined, value: unknown, refresh: () => void): void {
  if (!actionId) return;
  const bare = actionId.startsWith(`${tab.id}:`) ? actionId.slice(tab.id.length + 1) : actionId;
  void serviceCall<{ close?: boolean; refresh?: boolean }>(s, 'work-tray.action', { tab: tab.id, actionId: bare, value: value ?? null }, tab.owner ?? undefined)
    .then((reply) => (reply?.refresh ? refresh() : undefined))
    .catch(() => refresh());
}

/**
 * G-08: strips TUI-only non-actionable keyboard legends inside the modal's work
 * tray and disables `Stop all` when nothing in the tab is running.
 */
export function sanitizeTrayBody(node: unknown, hasRunning: boolean): unknown {
  if (!node || typeof node !== 'object' || Array.isArray(node)) return node;
  const o = node as Record<string, unknown>;
  if (o.t === 'KeyHints') {
    const hints = Array.isArray(o.hints) ? o.hints.filter((h) => h && typeof h === 'object' && !!(h as { action?: unknown }).action) : [];
    return hints.length ? { ...o, hints } : null;
  }
  if (o.t === 'Button' && !hasRunning && (o.onPress === 'stopall' || o.action === 'stopall' || o.label === 'Stop all')) {
    const { tone: _tone, variant: _variant, ...rest } = o;
    return { ...rest, disabled: true };
  }
  if (Array.isArray(o.children)) {
    const children = o.children.map((c) => sanitizeTrayBody(c, hasRunning)).filter((c) => c !== null);
    return { ...o, children };
  }
  return o;
}

/** The entry-actions facade that routes tree actions to the tab's owner. */
function trayEntryActions(s: SessionHandle, tab: TrayTab, refresh: () => void): EntryActions {
  return {
    entryId: `tray:${tab.id}`,
    target: { scope: 'session', hostId: s.hostId, sessionId: s.sessionId() } as UiTarget,
    send: (actionId, value) => trayAction(s, tab, actionId, value, refresh),
    registerValue: () => () => {},
    values: () => ({}),
    visible: () => {},
  };
}

/** The Running data of one session, refetched on its stream's moves (throttled). Shared by the
 * modal's tab badge and the tab body, so both count the same snapshot. The subagent records
 * follow the sticky `unipi:subagents:list` topic (live, no round trip) once it is there. */
export function useRunningData(s: () => SessionHandle): { data: Accessor<RunningData | undefined>; refetch: () => void } {
  const [data, { refetch }] = createResource(liveSource(s), async (state) => (state === 'live' ? fetchRunning(s()) : undefined));
  // Live sessions keep moving: refresh on run-state changes and topic/agent events (seq), throttled.
  let last = 0;
  createEffect(() => {
    void s().state.seq;
    if (Date.now() - last >= 2000) {
      last = Date.now();
      refetch();
    }
  });
  // Subscribe the records topic once per (re)connection (the Capsule does too; idempotent).
  createEffect(() => {
    if (s().state.conn === 'live') s().subscribeTopics([SUBAGENTS_LIST_TOPIC, ...bgTasksTopics(s().sessionId())]);
  });
  const merged = createMemo(() => {
    const d = data.latest;
    const topic = parseSubagentRuns(scopedSubagents(s().state.topics[SUBAGENTS_LIST_TOPIC]?.data, s().sessionId()));
    const bg = parseBgTasks(bgTopicData(s().state.topics, s().sessionId()));
    return d ? { ...d, ...(topic ? { runs: topic } : {}), ...(bg ? { bg } : {}) } : d;
  });
  return { data: merged, refetch: () => void refetch() };
}

/** How much runs right now (the tab badge): shells + live agents + sidekick + running tray tabs. */
export const runningBadgeCount = (data: RunningData | undefined): number => runningTotal(runningCounts(data));

/** "19:40" (today) or "Nov 1, 19:40" — a row's start time. */
export function startedText(at: number, now = Date.now()): string {
  const d = new Date(at);
  const time = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
  return new Date(now).toDateString() === d.toDateString() ? time : `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}, ${time}`;
}

/** One Running section: icon + title + count, an optional tool, then its rows. */
function Section(props: { icon: JSX.Element; title: string; count?: number; tool?: JSX.Element; testId?: string; children: JSX.Element }) {
  return (
    <section class="sm-sec" data-testid={props.testId}>
      <header class="sm-sec-head">
        <span class="sm-sec-icon">{props.icon}</span>
        <h3 class="sm-sec-title">{props.title}</h3>
        <Show when={(props.count ?? 0) > 0}>
          <span class="sm-sec-count" title={`${props.count} running`}>{props.count}</span>
        </Show>
        <span class="sm-grow" />
        {props.tool}
      </header>
      {props.children}
    </section>
  );
}

export { RunStatusIcon };

/** Rows past which the Subagents list windows (a big graph run). */
export const RUNS_VIRTUAL_AT = 30;
/** Fixed row height (px) — two lines; the windowed list relies on it. */
export const RUN_ROW_H = 60;
/** Rows visible in the windowed box. */
const RUNS_BOX_ROWS = 8;

type RunActions = {
  stop: (r: RunRow) => void;
  toBackground: (r: RunRow) => void;
  open: (r: RunRow) => void;
};

function RunRowView(props: { row: RunRow; now: number; actions: RunActions }) {
  const r = () => props.row;
  const dur = () => rowDurationMs(r(), props.now);
  const chip = () => profileChip(r().profile);
  const sub = (): RowStat[] => [
    ...statParts(r()),
    ...(r().status === 'running' && r().background ? [{ key: 'bg', text: 'In background' }] : []),
    ...(r().status === 'failed' && r().error ? [{ key: 'error', text: r().error!, title: r().error, tone: 'error' as const }] : []),
  ];
  return (
    <WorkRow
      testId="sm-run-row"
      dataRun={r().key}
      status={r().status}
      title={r().title}
      chips={chip() ? [{ text: chip()!, title: r().profile }] : []}
      {...(r().model ? { model: { text: shortModel(r().model), title: r().model } } : {})}
      sub={sub()}
      {...(dur() !== undefined ? { elapsed: { text: formatDurationMs(dur()!), title: isLive(r()) ? 'Running for' : 'Took' } } : {})}
      actions={
        <Show when={r().status === 'running' && !r().background && r().kind === 'record'}>
          <button type="button" onClick={(e) => { e.stopPropagation(); props.actions.toBackground(r()); }} aria-label="To background" title="Keep it running without waiting for it">
            <ArrowDownToLine size={14} />
            <span class="sm-btn-label">To background</span>
          </button>
        </Show>
      }
      {...(isLive(r()) ? { stop: { onStop: () => props.actions.stop(r()), title: 'Stop this run' } } : {})}
      open={{ onOpen: () => props.actions.open(r()), label: 'Open transcript' }}
    />
  );
}

/** The run rows: all of them up to {@link RUNS_VIRTUAL_AT}, else a scrolling box that mounts only the rows in view. */
function RunList(props: { rows: RunRow[]; now: number; actions: RunActions }) {
  const byKey = createMemo(() => new Map(props.rows.map((r) => [r.key, r] as const)));
  // Keyed by run: a topic update re-renders a row's fields, never remounts it.
  const keys = createMemo(() => props.rows.map((r) => r.key), undefined, { equals: (a, b) => a.length === b.length && a.every((k, i) => k === b[i]) });
  const windowed = () => props.rows.length > RUNS_VIRTUAL_AT;
  const [scroller, setScroller] = createSignal<HTMLElement>();
  const virt = useVirtual({ count: () => (windowed() ? props.rows.length : 0), scroller, estimate: () => RUN_ROW_H, key: (i) => props.rows[i]?.key ?? i, fixed: true, overscan: 6 });
  const row = (key: string) => (
    <Show when={byKey().get(key)}>{(r) => <RunRowView row={r()} now={props.now} actions={props.actions} />}</Show>
  );
  return (
    <Show
      when={windowed()}
      fallback={
        <div class="sm-runs" role="list" aria-label="Subagent runs">
          <For each={keys()}>{(k) => row(k)}</For>
        </div>
      }
    >
      <div class="sm-runs sm-runs-scroll" role="list" aria-label="Subagent runs" ref={setScroller} style={{ height: `${RUNS_BOX_ROWS * RUN_ROW_H}px` }} data-testid="sm-runs-scroll">
        <VirtualWindow virt={virt} fixed>
          {(i) => row(props.rows[i]?.key ?? '')}
        </VirtualWindow>
      </div>
    </Show>
  );
}

function SubagentsSection(props: { s: SessionHandle; rows: RunRow[]; now: number; refetch: () => void }) {
  // Owner 2026-11-06: the subagent filter opens on "Running" (in progress).
  const [filter, setFilter] = createSignal<RunFilter>('running');
  const counts = createMemo(() => filterCounts(props.rows));
  const shown = createMemo(() => props.rows.filter((r) => matchesFilter(r, filter())));
  const live = () => counts().running;
  const anyForeground = () => props.rows.some((r) => r.status === 'running' && !r.background && r.kind === 'record');
  const s = () => props.s;
  const done = () => props.refetch();
  // Every foreground run → background: the subagents' shortcut (Alt+B in the app).
  const allToBackground = () => void s().cmd({ type: '_oino/invoke_shortcut', shortcutId: 'unipi.subagents.foreground' }).then(done).catch(() => {});
  const abortAgent = (id: string) => s().cmd({ type: '_oino/agent_abort', agentId: id });
  const actions: RunActions = {
    stop: (r) => {
      const p =
        r.kind === 'record'
          ? serviceCall<{ cancelled?: boolean }>(s(), 'subagents.cancel', { id: r.id }, SUBAGENTS_EXT).then((v) => (v?.cancelled || !r.agentId ? undefined : abortAgent(r.agentId)))
          : abortAgent(r.id);
      void Promise.resolve(p)
        .catch(() => (r.agentId ? abortAgent(r.agentId) : undefined))
        .then(done)
        .catch(() => {});
    },
    toBackground: (r) => {
      // One run (`subagents.background`); an older extension only has the all-runs shortcut.
      void serviceCall(s(), 'subagents.background', { id: r.id }, SUBAGENTS_EXT)
        .catch(() => s().cmd({ type: '_oino/invoke_shortcut', shortcutId: 'unipi.subagents.foreground' }))
        .then(done)
        .catch(() => {});
    },
    open: (r) => {
      const cwd = String(s().state.info?.cwd ?? '') || undefined;
      setSessionModalTarget(undefined);
      openSubagentTab({ hostId: s().hostId, sessionId: s().sessionId(), ...(cwd ? { cwd } : {}) }, { agentId: r.agentId ?? r.id, name: r.title });
    },
  };
  return (
    <Section
      icon={<Bot size={14} />}
      title="Subagents"
      count={live()}
      testId="sm-sec-agents"
      tool={
        <Show when={anyForeground()}>
          <button type="button" class="sm-sec-tool sm-sec-text" onClick={allToBackground} title="Keep every foreground run going without waiting (Alt+B)">
            All to background
          </button>
        </Show>
      }
    >
      <div class="sm-runs-none" data-testid="sm-agents-summary">{live() > 0 ? `${live()} running · ${props.rows.length} total` : counts().done === props.rows.length ? `All ${props.rows.length} done` : `${props.rows.length} total`}</div>
      <FilterChips counts={counts()} filter={filter()} onFilter={setFilter} label="Filter runs by status" />
      <Show when={shown().length > 0} fallback={<div class="sm-runs-none">No {RUN_FILTERS.find((f) => f.id === filter())?.label.toLowerCase()} runs</div>}>
        <RunList rows={shown()} now={props.now} actions={actions} />
      </Show>
    </Section>
  );
}


/** Tasks the user dismissed (`bg:dismiss*`): the extension forgets them in its tray only, the
 * `unipi:bg:tasks` topic still lists them — so the view hides them itself. */
const [dismissed, setDismissed] = createSignal<ReadonlySet<string>>(new Set());
const dismissKey = (s: SessionHandle, id: string) => `${s.hostId}:${s.sessionId()}:${id}`;

/** The BG tab's identity for `work-tray.action` (the owner routes `bg:<verb>[:<id>]`). */
const BG_TRAY: TrayTab = { id: BG_TAB, label: 'Background tasks', order: 10, counts: { total: 0, running: 0 }, owner: BG_EXT };

/** Background tasks: the same row, filter chips and actions as Subagents — shell tasks of the chat. */
function BackgroundSection(props: { s: SessionHandle; rows: BgRow[]; now: number; refetch: () => void }) {
  const s = () => props.s;
  const [filter, setFilter] = createSignal<RunFilter>('all');
  const [confirmAll, setConfirmAll] = createSignal(false);
  const rows = createMemo(() => props.rows.filter((r) => !dismissed().has(dismissKey(s(), r.id))));
  const counts = createMemo(() => filterCounts(rows()));
  const shown = createMemo(() => rows().filter((r) => matchesFilter(r, filter())));
  const finished = () => rows().filter((r) => !isLive(r));
  const act = (actionId: string) => trayAction(s(), BG_TRAY, actionId, null, props.refetch);
  const open = (r: BgRow) => {
    const cwd = String(s().state.info?.cwd ?? '') || undefined;
    setSessionModalTarget(undefined);
    openBgTaskTab({ hostId: s().hostId, sessionId: s().sessionId(), ...(cwd ? { cwd } : {}) }, { taskId: r.id, name: r.title });
  };
  // The extension arms "stop all" with a first press and runs it on the second.
  const stopAll = () => {
    if (!confirmAll()) {
      setConfirmAll(true);
      setTimeout(() => setConfirmAll(false), 4000);
      return;
    }
    setConfirmAll(false);
    act('stopall');
    act('stopall');
  };
  const dismissFinished = () => {
    const ids = finished().map((r) => dismissKey(s(), r.id));
    setDismissed((cur) => new Set([...cur, ...ids]));
    act('dismissall');
  };
  return (
    <Section
      icon={<Terminal size={14} />}
      title="Background tasks"
      count={counts().running}
      testId="sm-sec-bg"
      tool={
        <>
          <Show when={counts().running > 0}>
            <button type="button" class="sm-sec-tool sm-sec-text" data-confirm={confirmAll() ? '' : undefined} onClick={stopAll} title="Stop every running task" data-testid="sm-bg-stopall">
              {confirmAll() ? 'Confirm stop all' : counts().running > 1 ? 'Stop all' : 'Stop'}
            </button>
          </Show>
          <Show when={finished().length > 0}>
            <button type="button" class="sm-sec-tool sm-sec-text" onClick={dismissFinished} title="Remove finished tasks from this list" data-testid="sm-bg-dismiss">
              Dismiss finished
            </button>
          </Show>
        </>
      }
    >
      <FilterChips counts={counts()} filter={filter()} onFilter={setFilter} label="Filter tasks by status" testPrefix="sm-bg-filter" />
      <Show when={shown().length > 0} fallback={<div class="sm-runs-none">No {RUN_FILTERS.find((f) => f.id === filter())?.label.toLowerCase()} tasks</div>}>
        <div class="sm-runs" role="list" aria-label="Background tasks">
          <For each={shown()}>
            {(r) => {
              const dur = () => (r.startedAt ? Math.max(0, (isLive(r) ? props.now : (r.endedAt ?? props.now)) - r.startedAt) : undefined);
              const trailing = () => [bgExitText(r), r.bytes > 0 ? formatBytes(r.bytes) : undefined].filter(Boolean).join(' · ');
              return (
                <WorkRow
                  testId="sm-bg-row"
                  dataRun={r.key}
                  status={r.status}
                  title={r.title}
                  chips={[{ text: r.isAgent ? 'Agent' : 'Shell', key: 'kind' }, ...(r.wakes && r.status === 'running' ? [{ text: 'wakes agent', key: 'wakes', title: 'A terminal notification wakes the agent' }] : [])]}
                  subText={r.command && r.command !== r.title ? r.command : (r.error ?? '')}
                  {...(dur() !== undefined ? { elapsed: { text: formatDurationMs(dur()!), title: isLive(r) ? 'Running for' : 'Took' } } : {})}
                  trailing={trailing()}
                  {...(r.status === 'running' ? { stop: { onStop: () => act(`stop:${r.id}`), title: 'Stop this task' } } : {})}
                  open={{ onOpen: () => open(r), label: 'Open live output' }}
                />
              );
            }}
          </For>
        </div>
      </Show>
    </Section>
  );
}

export function RunningTab(props: { s: SessionHandle; running?: { data: Accessor<RunningData | undefined>; refetch: () => void } }) {
  const s = () => props.s;
  const now = useClock();
  // The modal passes its shared snapshot (the tab badge counts the same); standalone use fetches its own.
  const own = props.running ? undefined : useRunningData(s);
  const data = () => (props.running ?? own!).data();
  const refetch = () => (props.running ?? own!).refetch();
  const runs = createMemo(() => runRowsOf(data()));
  const bash = () => data()?.bash ?? [];
  const bgRows = () => data()?.bg;
  // The native Background section stands in for the extension's tray tree (same tasks, one look).
  const tabs = () => visibleTrayTabsOf(data()?.tabs ?? []).filter((t) => !(bgRows() && t.id === BG_TAB && t.owner === BG_EXT));
  const nothing = () => !!data() && runs().length === 0 && (bgRows()?.length ?? 0) === 0 && bash().length === 0 && !data()?.sidekick && tabs().length === 0;

  return (
    <div class="sm-running" data-testid="sm-running">
      {/* First fetch (it asks every loaded extension; a cold one takes a while): a skeleton, not a blank tab. */}
      <Show when={!data()}>
        <div class="sm-skeleton" data-testid="sm-running-loading" aria-busy="true" aria-label="Loading what runs">
          <span style={{ width: '42%' }} />
          <span style={{ width: '88%' }} />
          <span style={{ width: '64%' }} />
        </div>
      </Show>
      <Show when={nothing()}>
        <div class="sm-nothing" data-testid="sm-nothing">
          <CircleCheck size={16} />
          <span class="sm-nothing-text">Nothing running</span>
          <span class="sm-grow" />
          <button type="button" class="sm-sec-tool sm-sec-icon-btn" onClick={() => refetch()} aria-label="Refresh" title="Refresh">
            <RefreshCw size={13} />
          </button>
        </div>
      </Show>

      <Show when={bash().length > 0}>
        <Section
          icon={<SquareTerminal size={14} />}
          title="Bash"
          count={bash().length}
          testId="sm-sec-bash"
          tool={
            <button type="button" class="sm-sec-tool sm-sec-icon-btn" onClick={() => refetch()} aria-label="Refresh" title="Refresh">
              <RefreshCw size={13} />
            </button>
          }
        >
          <div class="sm-list">
            <For each={bash()}>
              {(call) => (
                <div class="sm-row" data-testid="sm-bash-row">
                  <span class="sm-run-icon" data-status="running" role="img" aria-label="Running">
                    <LoaderCircle size={15} class="sm-spin" />
                  </span>
                  <span class="sm-row-main">
                    <span class="sm-row-title" title={call.command}>{call.command}</span>
                    <span class="sm-row-sub">
                      Started {startedText(call.startedAt, now())}
                      {call.pid ? ` · pid ${call.pid}` : ''}
                    </span>
                  </span>
                  <span class="sm-elapsed" title="Running for">{formatDurationMs(now() - call.startedAt)}</span>
                  <span class="sm-row-actions">
                    <Show when={canDetachBash(s(), bash().length)}>
                      <button type="button" onClick={() => void detachBash(s()).then(() => refetch()).catch(() => {})}>
                        Send to background
                      </button>
                    </Show>
                    <button type="button" data-tone="danger" onClick={() => void stopRun(s()).then(() => refetch()).catch(() => {})}>
                      Stop run
                    </button>
                  </span>
                </div>
              )}
            </For>
          </div>
        </Section>
      </Show>

      <Show when={runs().length > 0}>
        <SubagentsSection s={s()} rows={runs()} now={now()} refetch={refetch} />
      </Show>

      <Show when={(bgRows()?.length ?? 0) > 0}>
        <BackgroundSection s={s()} rows={bgRows()!} now={now()} refetch={refetch} />
      </Show>

      <Show when={data()?.sidekick}>
        {(sk) => (
          <Section icon={<Sparkles size={14} />} title="Sidekick" count={1} testId="sm-sec-sidekick">
            <div class="sm-list">
              <div class="sm-row">
                <span class="sm-run-icon" data-status="running" role="img" aria-label="Running">
                  <LoaderCircle size={15} class="sm-spin" />
                </span>
                <span class="sm-row-main">
                  <span class="sm-row-title sm-row-title-text">{sk().label}</span>
                  <span class="sm-row-sub">{sk().detail}</span>
                </span>
              </div>
            </div>
          </Section>
        )}
      </Show>

      <Show when={tabs().length > 0}>
        <Section icon={<Layers size={14} />} title="Work tray" count={tabs().reduce((n, t) => n + t.counts.running, 0)} testId="sm-sec-tray">
          <div class="sm-trees">
            <For each={tabs()}>
              {(tab) => (
                <div class="sm-tree" data-testid={`sm-tray-${tab.id}`}>
                  <div class="sm-tree-head">
                    <span class="sm-tree-label">{tab.label}</span>
                    <span class="sm-grow" />
                    <span class="sm-tree-count">
                      {tab.counts.running > 0 ? `${tab.counts.running} running · ${tab.counts.total}` : `${tab.counts.total}`}
                    </span>
                  </div>
                  <EntryContext.Provider value={trayEntryActions(s(), tab, refetch)}>
                    <NodeView node={sanitizeTrayBody(tab.body, tab.counts.running > 0) as never} path={[tab.id]} mode="auto" />
                  </EntryContext.Provider>
                </div>
              )}
            </For>
          </div>
        </Section>
      </Show>
    </div>
  );
}
