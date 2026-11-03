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
import { useClock as sharedClock } from '../../lib/clock';
import { createEffect, createMemo, createResource, createSignal, For, Show, type Accessor, type JSX } from 'solid-js';
import { ArrowDownToLine, Ban, Bot, Check, CircleCheck, Layers, LoaderCircle, Moon, PanelRightOpen, RefreshCw, Sparkles, Square, SquareTerminal, X } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { EntryContext, type EntryActions, type UiTarget } from '../../ui/actions';
import { NodeView } from '../../ui/NodeView';
import { useVirtual, VirtualWindow } from '../../ui/virtual';
import { formatDurationMs, liveSource, parseAgents, parseExtensions, parseRunningBash, parseSubagentRuns, parseTrayTabs, serviceCall, type AgentInfo, type SubagentRun, type TrayTab } from './modalData';
import { runningCounts, runningTotal, runRowsOf, SUBAGENTS_LIST_TOPIC, visibleTrayTabsOf, type RunningData } from '../../lib/status/running';
import { filterCounts, isLive, matchesFilter, profileChip, ROW_STATUS_LABEL, RUN_FILTERS, rowDurationMs, shortModel, statParts, SUBAGENTS_EXT, type RunFilter, type RunRow, type RunRowStatus } from '../../lib/status/subagentRows';
import { openSubagentTab } from '../panes/tabs/sessionTabs';
import { setSessionModalTarget } from './modalTarget';

export { agentSeenAt } from '../../lib/status/running';

/** Now, ticking once a second while the tab is mounted (durations). */
export function useClock(): Accessor<number> {
  return sharedClock();
}

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
export async function fetchRunning(s: SessionHandle): Promise<{ bash: ReturnType<typeof parseRunningBash>; agents: AgentInfo[]; runs?: SubagentRun[]; sidekick?: { label: string; detail: string }; tabs: TrayTab[] }> {
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
                runs = parseSubagentRuns(v);
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
    if (s().state.conn === 'live') s().subscribeTopics([SUBAGENTS_LIST_TOPIC]);
  });
  const merged = createMemo(() => {
    const d = data.latest;
    const topic = parseSubagentRuns(s().state.topics[SUBAGENTS_LIST_TOPIC]?.data);
    return d && topic ? { ...d, runs: topic } : d;
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
  return (
    <div class="sm-run" data-status={r().status} data-testid="sm-run-row" data-run={r().key} role="listitem">
      <RunStatusIcon status={r().status} />
      <div class="sm-run-main">
        <div class="sm-run-line">
          <span class="sm-run-title" title={r().title}>{r().title}</span>
          <Show when={chip()}>
            <span class="sm-run-chip" title={r().profile}>{chip()}</span>
          </Show>
          <Show when={r().model}>
            <span class="sm-run-model" title={r().model}>{shortModel(r().model)}</span>
          </Show>
        </div>
        <div class="sm-run-sub">
          <For each={statParts(r())}>{(p) => <span class="sm-run-stat" data-stat={p.key} title={p.title}>{p.text}</span>}</For>
          <Show when={r().status === 'running' && r().background}>
            <span class="sm-run-stat">In background</span>
          </Show>
          <Show when={r().status === 'failed' && r().error}>
            <span class="sm-run-stat sm-run-err" title={r().error}>{r().error}</span>
          </Show>
        </div>
      </div>
      <Show when={dur() !== undefined}>
        <span class="sm-elapsed" title={isLive(r()) ? 'Running for' : 'Took'}>{formatDurationMs(dur()!)}</span>
      </Show>
      <span class="sm-row-actions sm-run-actions">
        <Show when={r().status === 'running' && !r().background && r().kind === 'record'}>
          <button type="button" onClick={() => props.actions.toBackground(r())} aria-label="To background" title="Keep it running without waiting for it">
            <ArrowDownToLine size={14} />
            <span class="sm-btn-label">To background</span>
          </button>
        </Show>
        <Show when={isLive(r())}>
          <button type="button" data-tone="danger" onClick={() => props.actions.stop(r())} aria-label="Stop" title="Stop this run">
            <Square size={13} />
            <span class="sm-btn-label">Stop</span>
          </button>
        </Show>
        <button type="button" class="sm-icon-btn" onClick={() => props.actions.open(r())} aria-label="Open transcript" title="Open transcript">
          <PanelRightOpen size={15} />
        </button>
      </span>
    </div>
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
  const [filter, setFilter] = createSignal<RunFilter>('all');
  const counts = createMemo(() => filterCounts(props.rows));
  const shown = createMemo(() => props.rows.filter((r) => matchesFilter(r, filter())));
  const chips = () => RUN_FILTERS.filter((f) => f.id !== 'cancelled' || counts().cancelled > 0 || filter() === 'cancelled');
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
          <button type="button" class="sm-sec-tool" onClick={allToBackground} title="Keep every foreground run going without waiting (Alt+B)">
            All to background
          </button>
        </Show>
      }
    >
      <div class="sm-chips" role="toolbar" aria-label="Filter runs by status">
        <For each={chips()}>
          {(f) => (
            <button type="button" class="sm-chip" aria-pressed={filter() === f.id} data-filter={f.id} data-testid={`sm-filter-${f.id}`} onClick={() => setFilter(filter() === f.id && f.id !== 'all' ? 'all' : f.id)}>
              {f.label}
              <span class="sm-chip-n">{counts()[f.id]}</span>
            </button>
          )}
        </For>
      </div>
      <Show when={shown().length > 0} fallback={<div class="sm-runs-none">No {RUN_FILTERS.find((f) => f.id === filter())?.label.toLowerCase()} runs</div>}>
        <RunList rows={shown()} now={props.now} actions={actions} />
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
  const tabs = () => visibleTrayTabsOf(data()?.tabs ?? []);
  const nothing = () => !!data() && runs().length === 0 && bash().length === 0 && !data()?.sidekick && tabs().length === 0;

  return (
    <div class="sm-running" data-testid="sm-running">
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
