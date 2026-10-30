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
import { For, Show, createEffect, createResource, createSignal, onCleanup, type Accessor } from 'solid-js';
import { Bot, ChevronRight, RefreshCw, SquareTerminal } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { EntryContext, type EntryActions, type UiTarget } from '../../ui/actions';
import { NodeView } from '../../ui/NodeView';
import { formatDurationMs, liveSource, parseAgents, parseExtensions, parseRunningBash, parseTrayTabs, serviceCall, type AgentInfo, type TrayTab } from './modalData';
import { liveAgentsOf, visibleTrayTabsOf } from '../../lib/status/running';

/** Now, ticking once a second while the tab is mounted (durations). */
export function useClock(): Accessor<number> {
  const [now, setNow] = createSignal(Date.now());
  const t = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(t));
  return now;
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

export function RunningTab(props: { s: SessionHandle }) {
  const s = () => props.s;
  const now = useClock();
  const [data, { refetch }] = createResource(liveSource(s), async (state) => (state === 'live' ? fetchRunning(s()) : undefined));
  // Live sessions keep moving: refresh on run-state changes and topic/agent events (seq), throttled by the resource's own dedupe.
  let last = 0;
  createEffect(() => {
    void s().state.seq;
    if (Date.now() - last >= 2000) {
      last = Date.now();
      refetch();
    }
  });
  const agents = () => data.latest?.agents ?? [];
  const liveAgents = () => liveAgentsOf(agents());
  const bash = () => data.latest?.bash ?? [];
  const tabs = () => visibleTrayTabsOf(data.latest?.tabs ?? []);
  const toBackground = () => void s().cmd({ type: '_oino/invoke_shortcut', shortcutId: 'unipi.subagents.foreground' }).catch(() => {});
  const stopAgent = (id: string) => void s().cmd({ type: '_oino/agent_abort', agentId: id }).then(() => refetch()).catch(() => {});

  return (
    <div data-testid="sm-running">
      <div class="sm-section-title">
        <SquareTerminal size={13} /> Bash <span class="sm-grow" />
        <button type="button" onClick={() => refetch()} aria-label="Refresh">
          <RefreshCw size={13} />
        </button>
      </div>
      <Show when={bash().length > 0} fallback={<p class="sm-note">No shell call is running.</p>}>
        <div class="sm-rows">
          <For each={bash()}>
            {(call) => (
              <div class="sm-row" data-testid="sm-bash-row">
                <span class="sm-dot" data-state="running" />
                <span class="sm-row-main">
                  <span class="sm-row-title">{call.command}</span>
                  <span class="sm-row-sub">
                    {formatDurationMs(now() - call.startedAt)}
                    {call.pid ? ` · pid ${call.pid}` : ''}
                  </span>
                </span>
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

      <div class="sm-section-title">
        <Bot size={13} /> Subagents <span class="sm-grow" />
        <Show when={liveAgents().length > 0}>
          <button type="button" onClick={toBackground} title="Alt+B">All to background</button>
        </Show>
      </div>
      <Show when={agents().length > 0} fallback={<p class="sm-note">No subagents.</p>}>
        <div class="sm-rows">
          <For each={agents()}>
            {(a) => (
              <div class="sm-row" classList={{ [`sm-indent-${Math.min(2, Math.max(0, a.depth - 1))}`]: a.depth > 1 }} data-testid="sm-agent-row">
                <span class="sm-dot" data-state={a.state} />
                <span class="sm-row-main">
                  <span class="sm-row-title" style={{ 'font-family': 'inherit' }}>{a.label}</span>
                  <span class="sm-row-sub">
                    {a.role || a.owner} · {a.state}
                  </span>
                </span>
                <span class="sm-row-actions">
                  <Show when={a.state === 'running' || a.state === 'idle'}>
                    <button type="button" data-tone="danger" onClick={() => stopAgent(a.id)}>
                      Stop
                    </button>
                  </Show>
                </span>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={data.latest?.sidekick}>
        {(sk) => (
          <>
            <div class="sm-section-title">
              <ChevronRight size={13} /> Sidekick
            </div>
            <div class="sm-rows">
              <div class="sm-row">
                <span class="sm-dot" data-state="running" />
                <span class="sm-row-main">
                  <span class="sm-row-title" style={{ 'font-family': 'inherit' }}>{sk().label}</span>
                  <span class="sm-row-sub">{sk().detail}</span>
                </span>
              </div>
            </div>
          </>
        )}
      </Show>

      <Show when={tabs().length > 0}>
        <div class="sm-section-title">
          <ChevronRight size={13} /> Work tray
        </div>
        <For each={tabs()}>
          {(tab) => (
            <div class="sm-tree" data-testid={`sm-tray-${tab.id}`}>
              <div class="sm-section-title" style={{ margin: '2px 0 6px' }}>
                {tab.label} <span class="sm-grow" />
                <span class="sm-tree-count">
                  {tab.counts.running > 0 ? `${tab.counts.running} running · ${tab.counts.total}` : `${tab.counts.total}`}
                </span>
              </div>
              <EntryContext.Provider value={trayEntryActions(s(), tab, refetch)}>
                <NodeView node={tab.body as never} path={[tab.id]} mode="auto" />
              </EntryContext.Provider>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
