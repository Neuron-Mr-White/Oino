/**
 * Running tab (WP-A17, Z-95): **the** full Running view —
 * - foreground bash calls (`_oino/running_bash`) with Stop and "Send to
 *   background" (the watchdog's `/bg-detach` command, which adopts the call as
 *   an `unipi.background-tasks` task — the prompt RPC runs `/<command>` without
 *   a model turn or an outbox bubble);
 * - the nested-agent tree (`_oino/agents`) with Stop per agent and
 *   "All to background" (`_oino/invoke_shortcut`, the subagents' Alt+B);
 * - the fusion sidekick (`fusion.status`) when a handoff is live;
 * - every `work-tray.tab` registration (bg tasks, dream, …) rendered as its
 *   OUS tree through U9 `NodeView` (G-20), actions routed to the tab's owner
 *   through `work-tray.action` — the tray panel itself is never opened.
 */
import { useClock as sharedClock } from '../../lib/clock';
import { For, Show, createEffect, createResource, createSignal, onCleanup, type Accessor, type JSX } from 'solid-js';
import { Bot, CircleCheck, Layers, RefreshCw, Sparkles, SquareTerminal } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { EntryContext, type EntryActions, type UiTarget } from '../../ui/actions';
import { NodeView } from '../../ui/NodeView';
import { formatDurationMs, liveSource, parseAgents, parseExtensions, parseRunningBash, parseTrayTabs, serviceCall, type AgentInfo, type TrayTab } from './modalData';
import { liveAgentsOf, runningCounts, runningTotal, visibleTrayTabsOf, type RunningData } from '../../lib/status/running';

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
 * One fetch pass: the native sections' data plus every loaded extension's
 * work-tray tabs. `work-tray.tab` is a fan-out service; the frontend's
 * `_oino/service_call` reaches one server, so the app asks each loaded
 * extension (unknown services answer `E_NOT_FOUND` and are dropped).
 */
export async function fetchRunning(s: SessionHandle): Promise<{ bash: ReturnType<typeof parseRunningBash>; agents: AgentInfo[]; sidekick?: { label: string; detail: string }; tabs: TrayTab[] }> {
  const [bash, agents, extensions] = await Promise.all([
    s.cmd<unknown>({ type: '_oino/running_bash' }).catch(() => undefined),
    s.cmd<unknown>({ type: '_oino/agents' }).catch(() => undefined),
    s.cmd<unknown>({ type: '_oino/extensions' }).catch(() => undefined),
  ]);
  const loaded = parseExtensions(extensions).filter((e) => e.loaded);
  const tabs: TrayTab[] = [];
  if (loaded.length > 0) {
    const replies = await Promise.all(
      loaded.map((e) =>
        serviceCall<unknown>(s, 'work-tray.tab', { open: true }, e.id)
          .then((v) => parseTrayTabs(v))
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
  return { bash: parseRunningBash(bash), agents: parseAgents(agents), sidekick, tabs };
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
 * modal's tab badge and the tab body, so both count the same snapshot. */
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
  return { data: () => data.latest, refetch: () => void refetch() };
}

/** How much runs right now (the tab badge): shells + live agents + sidekick + running tray tabs. */
export const runningBadgeCount = (data: RunningData | undefined): number => runningTotal(runningCounts(data));

/**
 * When the app first saw each agent (`_oino/agents` carries no start time): the agent row's
 * elapsed counts from here — exact for agents started while the app watched, a lower bound
 * otherwise. Keyed by session + agent id; kept for the app's lifetime (a handful of ids).
 */
const agentSeen = new Map<string, number>();
export function agentSeenAt(sessionKey: string, agentId: string, now = Date.now()): number {
  const key = `${sessionKey}\u0000${agentId}`;
  let at = agentSeen.get(key);
  if (at === undefined) {
    at = now;
    agentSeen.set(key, at);
  }
  return at;
}

/** "19:40" (today) or "Nov 1, 19:40" — a row's start time. */
export function startedText(at: number, now = Date.now()): string {
  const d = new Date(at);
  const time = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
  return new Date(now).toDateString() === d.toDateString() ? time : `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}, ${time}`;
}

const AGENT_STATE_LABEL: Record<AgentInfo['state'], string> = { running: 'Running', idle: 'Idle', hibernated: 'Hibernated', closed: 'Closed' };

/** One Running section: icon + title + count, an optional tool, then rows or its empty state. */
function Section(props: { icon: JSX.Element; title: string; count?: number; tool?: JSX.Element; empty?: string; testId?: string; children: JSX.Element }) {
  return (
    <section class="sm-sec" data-testid={props.testId}>
      <header class="sm-sec-head">
        <span class="sm-sec-icon">{props.icon}</span>
        <h3 class="sm-sec-title">{props.title}</h3>
        <Show when={(props.count ?? 0) > 0}>
          <span class="sm-sec-count">{props.count}</span>
        </Show>
        <span class="sm-grow" />
        {props.tool}
      </header>
      {props.children}
    </section>
  );
}

/** The quiet "Nothing running" line of an empty section. */
function EmptyLine(props: { text: string; hint?: string }) {
  return (
    <div class="sm-sec-empty">
      <CircleCheck size={16} />
      <span class="sm-sec-empty-text">{props.text}</span>
      <Show when={props.hint}>
        <span class="sm-sec-empty-hint">{props.hint}</span>
      </Show>
    </div>
  );
}

export function RunningTab(props: { s: SessionHandle; running?: { data: Accessor<RunningData | undefined>; refetch: () => void } }) {
  const s = () => props.s;
  const now = useClock();
  // The modal passes its shared snapshot (the tab badge counts the same); standalone use fetches its own.
  const own = props.running ? undefined : useRunningData(s);
  const data = () => (props.running ?? own!).data();
  const refetch = () => (props.running ?? own!).refetch();
  const agents = () => data()?.agents ?? [];
  const liveAgents = () => liveAgentsOf(agents());
  const bash = () => data()?.bash ?? [];
  const tabs = () => visibleTrayTabsOf(data()?.tabs ?? []);
  const toBackground = () => void s().cmd({ type: '_oino/invoke_shortcut', shortcutId: 'unipi.subagents.foreground' }).catch(() => {});
  const stopAgent = (id: string) => void s().cmd({ type: '_oino/agent_abort', agentId: id }).then(() => refetch()).catch(() => {});

  return (
    <div class="sm-running" data-testid="sm-running">
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
        <Show when={bash().length > 0} fallback={<EmptyLine text="Nothing running" hint="No shell command is running." />}>
          <div class="sm-list">
            <For each={bash()}>
              {(call) => (
                <div class="sm-row" data-testid="sm-bash-row">
                  <span class="sm-dot" data-state="running" />
                  <span class="sm-row-main">
                    <span class="sm-row-title" title={call.command}>{call.command}</span>
                    <span class="sm-row-sub">
                      Started {startedText(call.startedAt, now())}
                      {call.pid ? ` · pid ${call.pid}` : ''}
                    </span>
                  </span>
                  <span class="sm-elapsed" title="Elapsed">{formatDurationMs(now() - call.startedAt)}</span>
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
        </Show>
      </Section>

      <Section
        icon={<Bot size={14} />}
        title="Subagents"
        count={liveAgents().length}
        testId="sm-sec-agents"
        tool={
          <Show when={liveAgents().length > 0}>
            <button type="button" class="sm-sec-tool" onClick={toBackground} title="Alt+B">All to background</button>
          </Show>
        }
      >
        <Show when={agents().length > 0} fallback={<EmptyLine text="Nothing running" hint="No subagents in this chat." />}>
          <div class="sm-list">
            <For each={agents()}>
              {(a) => {
                const live = () => a.state === 'running' || a.state === 'idle';
                const seen = agentSeenAt(s().key, a.id);
                return (
                  <div class="sm-row" classList={{ [`sm-indent-${Math.min(2, Math.max(0, a.depth - 1))}`]: a.depth > 1 }} data-testid="sm-agent-row">
                    <span class="sm-dot" data-state={a.state} />
                    <span class="sm-row-main">
                      <span class="sm-row-title sm-row-title-text" title={a.label}>{a.label}</span>
                      <span class="sm-row-sub">{a.role || a.owner}</span>
                    </span>
                    <span class="sm-state-pill" data-state={a.state}>{AGENT_STATE_LABEL[a.state]}</span>
                    <Show when={live()}>
                      <span class="sm-elapsed" title="Running for at least">{formatDurationMs(now() - seen)}</span>
                    </Show>
                    <span class="sm-row-actions">
                      <Show when={live()}>
                        <button type="button" data-tone="danger" onClick={() => stopAgent(a.id)}>
                          Stop
                        </button>
                      </Show>
                    </span>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </Section>

      <Show when={data()?.sidekick}>
        {(sk) => (
          <Section icon={<Sparkles size={14} />} title="Sidekick" count={1} testId="sm-sec-sidekick">
            <div class="sm-list">
              <div class="sm-row">
                <span class="sm-dot" data-state="running" />
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
        <Section icon={<Layers size={14} />} title="Work tray" testId="sm-sec-tray">
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
