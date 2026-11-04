/**
 * Info tab (WP-A17, G-13): the `/info` pages natively — Session (identity +
 * `get_session_stats` + this session's usage; cross-session history is Settings › Usage), Tools, Skills (commands with source `skill`,
 * grouped like the extension groups them) and Modules (`_oino/extensions`
 * plus every extension's `info-screen.groups` status card) — plus the
 * watchdog Limits section (G-16).
 *
 * P1-03: Usage (per-day token/cost history from the session files) and Tools
 * (the active LLM tools) are data only the info-screen extension computes —
 * no host RPC carries them. Opening those pages asks the extension
 * (`/info usage|tools`) and renders *its* page from the dashboard's reserved
 * entry (the Tabs pane of that id), live as it repaints; this PC's hub totals
 * and the slash-command inventory stay as the quick native part.
 */
import { createEffect, createMemo, useContext, createResource, createSignal, For, on, Show } from 'solid-js';
import { NodeView } from '../../ui/NodeView';
import { EntryContext, createEntryActions, type UiTarget } from '../../ui/actions';
import { sessionUi } from '../../ui/registries';
import type { UiEntry, UiNode } from '../../ui/types';
import type { SessionHandle } from '../../lib/session/store';
import { hubs } from '../../lib/session/hub';
import { RouterContext, useNavigate } from '@solidjs/router';
import { ChevronRight } from 'lucide-solid';
import { formatCount, formatDurationMs, formatUsd, liveSource, sessionCostView, sessionModelUsage, sessionSpanMs, modelRowReplies, statsSpanMs, parseExtensions, parseSessionStats, serviceCall, type CommandInfo, type ExtensionInfo, type SessionStats } from './modalData';
import { LimitsSection } from './LimitsSection';
import { closeSessionModal, type InfoPage } from './SessionModal';

const PAGES: Array<{ id: InfoPage; label: string }> = [
  { id: 'session', label: 'Session' },
  { id: 'tools', label: 'Tools' },
  { id: 'skills', label: 'Skills' },
  { id: 'modules', label: 'Modules' },
];

/** A skill's group (mirrors the info-screen extension's grouping). */
export function skillGroup(c: CommandInfo): string {
  const info = (c.sourceInfo ?? {}) as Record<string, unknown>;
  const path = typeof info.path === 'string' ? info.path : '';
  if (info.scope === 'project') return 'project';
  if (path.includes('/.agents/')) return 'agents';
  if (typeof info.origin === 'string' && info.origin === 'package') return 'package';
  return 'user';
}

/** A command's source label. */
export function commandSource(c: CommandInfo): string {
  if (c.source === 'skill') return 'skill';
  const info = (c.sourceInfo ?? {}) as Record<string, unknown>;
  if (typeof info.source === 'string') return info.source.replace(/^extension:/, '');
  return c.source ?? 'builtin';
}

export function groupBy<T>(items: T[], key: (t: T) => string): Array<[string, T[]]> {
  const by = new Map<string, T[]>();
  for (const item of items) {
    const k = key(item);
    const list = by.get(k);
    if (list) list.push(item);
    else by.set(k, [item]);
  }
  return [...by.entries()].sort((a, b) => a[0].localeCompare(b[0]));
}

function Kv(props: { rows: Array<[string, string, boolean?]> }) {
  return (
    <div class="sm-kv">
      <For each={props.rows}>
        {([k, v, num]) => (
          <div class="sm-kv-row">
            <span class="sm-kv-key">{k}</span>
            <span class="sm-kv-val" data-num={num ? '' : undefined}>{v}</span>
          </div>
        )}
      </For>
    </div>
  );
}

/** This session's usage, compact: stat strip, token mix, per-model rows. Cross-session history lives in Settings › Usage. */
function SessionPage(props: { s: SessionHandle; stats: SessionStats | undefined }) {
  const s = () => props.s;
  // The modal also mounts without a router (tests, the gallery): fall back to the hash route.
  const navigate = useContext(RouterContext) ? useNavigate() : (to: string) => void (location.hash = `#${to}`);
  const card = () => hubs().ensure(s().hostId).state.cards[s().sessionId()];
  const state = () => s().state;
  const model = () => state().model;
  const stats = () => props.stats;
  // Whole-session host totals (same source as cost/tokens); only an older host without them falls back to the loaded window.
  const models = createMemo(() => stats()?.models ?? (stats() ? sessionModelUsage(state().entries) : []));
  const span = createMemo(() => (stats()?.firstAt !== undefined ? statsSpanMs(stats()) : stats() ? sessionSpanMs(state().entries) : undefined));
  const mix = () => {
    const t = stats()?.tokens;
    return t ? ([['In', t.input, 'var(--color-wm-cyan)'], ['Out', t.output, 'var(--color-claw)'], ['Cache read', t.cacheRead, 'var(--color-wm-green)'], ['Cache write', t.cacheWrite, 'var(--color-amber)']] as const) : [];
  };
  const total = () => stats()?.tokens.total || 1;
  return (
    <>
      <div class="sm-grid sm-grid-4">
        <div class="sm-stat" title={sessionCostView(stats()).title} data-testid="sm-info-cost">
          <span class="sm-stat-label">Cost</span>
          <span class="sm-stat-value">{sessionCostView(stats()).text}</span>
          <Show when={sessionCostView(stats()).sub}>{(sub) => <span class="sm-stat-sub">{sub()}</span>}</Show>
        </div>
        <div class="sm-stat" data-testid="sm-info-tokens"><span class="sm-stat-label">Tokens</span><span class="sm-stat-value">{formatCount(stats()?.tokens.total ?? 0)}</span></div>
        <div class="sm-stat" data-testid="sm-info-turns"><span class="sm-stat-label">Turns</span><span class="sm-stat-value">{stats()?.userMessages ?? 0}</span><span class="sm-stat-sub">{stats()?.assistantMessages ?? 0} replies · {stats()?.toolCalls ?? 0} tool calls</span></div>
        <div class="sm-stat" data-testid="sm-info-duration"><span class="sm-stat-label">Duration</span><span class="sm-stat-value">{span() === undefined ? '—' : formatDurationMs(span()!)}</span></div>
      </div>
      <Show when={(stats()?.tokens.total ?? 0) > 0}>
        <div class="sm-mix" data-testid="sm-info-mix">
          <div class="sm-ctx-stack" aria-hidden="true">
            <For each={mix()}>{([, n, color]) => <span style={{ width: `${(n / total()) * 100}%`, background: color }} />}</For>
          </div>
          <div class="sm-mix-legend">
            <For each={mix()}>
              {([label, n, color]) => (
                <span class="sm-mix-item"><i style={{ background: color }} />{label} <b>{formatCount(n)}</b></span>
              )}
            </For>
          </div>
        </div>
      </Show>
      <Show when={models().length > 0}>
        <div class="sm-section-title">Models</div>
        <div class="sm-rows" data-testid="sm-info-models">
          <For each={models()}>
            {(m) => (
              <div class="sm-row">
                <span class="sm-row-main">
                  <span class="sm-row-title" style={{ 'font-family': 'inherit' }}>{m.model}</span>
                  <span class="sm-row-sub">{modelRowReplies(m)} · {formatCount(m.input)} in · {formatCount(m.output)} out · {formatCount(m.cacheRead)} cached</span>
                </span>
                <span class="sm-elapsed" title={m.cost > 0 ? ((m.childCost ?? 0) > 0 ? `${formatUsd(m.childCost!)} of it in child runs (subagents, workers, memory, naming)` : undefined) : 'This model reports no price'}>{m.cost > 0 ? formatUsd(m.cost) : '—'}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Kv
        rows={[
          ['Session', state().sessionId || s().sessionId()],
          ['Name', state().name ?? card()?.name ?? '—'],
          ['Folder', card()?.cwd ?? (String(state().info?.cwd ?? '') || '—')],
          ['Model', model() ? `${model()!.name ?? model()!.id} (${model()!.provider ?? '?'})` : '—'],
          ['Thinking', state().thinkingLevel ?? '—'],
          ['Messages', String(state().entries.length)],
          ['Session file', stats()?.sessionFile ?? card()?.file ?? '—'],
        ]}
      />
      <button
        type="button"
        class="sm-link"
        data-testid="sm-info-all-usage"
        onClick={() => {
          closeSessionModal();
          navigate('/settings/usage');
        }}
      >
        Usage across all sessions <ChevronRight size={14} />
      </button>
    </>
  );
}

const DASHBOARD = 'unipi.info-screen/dashboard';

/** The info-screen dashboard entry of this session (present once `/info …` ran). */
function dashboardEntry(s: SessionHandle): UiEntry | undefined {
  return sessionUi(s.hostId, s.sessionId()).stack('overlay')().find((e) => e.id === DASHBOARD);
}

/** The extension's own page `page` from the dashboard's fallback Tabs tree. Pure. */
export function dashboardPane(entry: UiEntry | undefined, page: string): UiNode | undefined {
  const view = entry?.view as { kind?: string; fallback?: unknown; root?: unknown } | undefined;
  const tree = (view?.kind === 'reserved' ? view.fallback : view?.root) as { t?: string; panes?: Record<string, UiNode> } | undefined;
  if (!tree || tree.t !== 'Tabs') return undefined;
  const pane = tree.panes?.[page] as (UiNode & { t?: string }) | undefined;
  // An unloaded page is a bare spinner: nothing to show yet.
  return pane && !(pane.t === 'Spinner' && !(pane as { label?: string }).label) ? pane : undefined;
}

/**
 * P1-03: the info-screen extension's page `page`, asked for once per mount
 * (`/info <page>` repaints the dashboard entry with that page loaded) and
 * rendered from the entry live. No extension → a quiet note.
 */
function ExtPane(props: { s: SessionHandle; page: 'usage' | 'tools'; label: string }) {
  const has = () => props.s.state.commands.some((c) => c.name === 'info' || c.name === 'unipi:info');
  const command = () => (props.s.state.commands.some((c) => c.name === 'info') ? 'info' : 'unipi:info');
  createEffect(
    on(has, (ok) => {
      if (ok) props.s.send(`/${command()} ${props.page}`, { mode: 'prompt' });
    }),
  );
  const pane = createMemo(() => dashboardPane(dashboardEntry(props.s), props.page));
  const target = (): UiTarget => ({ scope: 'session', hostId: props.s.hostId, sessionId: props.s.sessionId() });
  const actions = createMemo(() => createEntryActions(DASHBOARD, target()));
  return (
    <div class="sm-ext-pane" data-testid={`sm-ext-${props.page}`}>
      <div class="sm-section-title">{props.label}</div>
      <Show
        when={pane()}
        fallback={<p class="sm-note">{has() ? 'Loading from the Info extension…' : 'Turn on the Info extension (Settings › Extensions) for this page.'}</p>}
      >
        {(node) => (
          <EntryContext.Provider value={actions()}>
            <div class="ui-entry sm-ext-tree" data-entry={DASHBOARD}>
              <NodeView node={node()} path={[props.page]} mode="auto" />
            </div>
          </EntryContext.Provider>
        )}
      </Show>
    </div>
  );
}

function ToolsPage(props: { s: SessionHandle; commands: CommandInfo[] }) {
  const groups = () => groupBy(props.commands.filter((c) => c.source !== 'skill'), commandSource);
  return (
    <>
    <ExtPane s={props.s} page="tools" label="Agent tools" />
    <div class="sm-section-title sm-section-gap">Commands</div>
    <Show when={groups().length > 0} fallback={<p class="sm-note">No commands.</p>}>
      <For each={groups()}>
        {([source, list]) => (
          <div>
            <div class="sm-section-title">{source} <span class="sm-grow" /> <span class="sm-tree-count">{list.length}</span></div>
            <div class="sm-rows">
              <For each={list}>
                {(c) => (
                  <div class="sm-row">
                    <span class="sm-row-main">
                      <span class="sm-row-title" style={{ 'font-family': 'inherit' }}>/{c.name}</span>
                      <span class="sm-row-sub">{c.description ?? ''}</span>
                    </span>
                  </div>
                )}
              </For>
            </div>
          </div>
        )}
      </For>
    </Show>
    </>
  );
}

function SkillsPage(props: { commands: CommandInfo[] }) {
  const skills = () => props.commands.filter((c) => c.source === 'skill');
  const groups = () => groupBy(skills(), skillGroup);
  return (
    <Show when={skills().length > 0} fallback={<p class="sm-note">No skills. Add them under ~/.oino/skills or .oino/skills.</p>}>
      <For each={groups()}>
        {([group, list]) => (
          <div>
            <div class="sm-section-title">{group} <span class="sm-grow" /> <span class="sm-tree-count">{list.length}</span></div>
            <div class="sm-rows">
              <For each={list}>
                {(c) => (
                  <div class="sm-row">
                    <span class="sm-row-main">
                      <span class="sm-row-title" style={{ 'font-family': 'inherit' }}>{c.name.replace(/^skill:/, '')}</span>
                      <span class="sm-row-sub">{c.description ?? ''}</span>
                    </span>
                  </div>
                )}
              </For>
            </div>
          </div>
        )}
      </For>
    </Show>
  );
}

/** One extension status card from `info-screen.groups` (id, name, stats, data). */
export type InfoGroup = { owner: string; id: string; name: string; stats: Array<{ id: string; label: string; value: string; detail?: string }> };

/** Decodes one `info-screen.groups` reply (a group or a list) into cards. Pure. */
export function parseInfoGroups(owner: string, reply: unknown): InfoGroup[] {
  const list = Array.isArray(reply) ? reply : reply && typeof reply === 'object' ? [reply] : [];
  const out: InfoGroup[] = [];
  for (const raw of list) {
    const g = raw as { id?: unknown; name?: unknown; config?: { stats?: unknown }; data?: Record<string, unknown> };
    if (typeof g?.id !== 'string') continue;
    const data = (g.data && typeof g.data === 'object' ? g.data : {}) as Record<string, { value?: unknown; detail?: unknown } | undefined>;
    const declared = Array.isArray(g.config?.stats) ? (g.config!.stats as Array<{ id?: unknown; label?: unknown; show?: unknown }>) : [];
    const ids = declared.length
      ? declared.filter((st) => typeof st.id === 'string' && st.show !== false).map((st) => ({ id: st.id as string, label: typeof st.label === 'string' ? st.label : (st.id as string) }))
      : Object.keys(data).filter((k) => k !== 'raw').map((k) => ({ id: k, label: k }));
    const stats = ids.flatMap(({ id, label }) => {
      const v = data[id];
      if (!v || v.value === undefined || v.value === null || v.value === '') return [];
      return [{ id, label, value: String(v.value), ...(typeof v.detail === 'string' && v.detail ? { detail: v.detail } : {}) }];
    });
    out.push({ owner, id: g.id, name: typeof g.name === 'string' ? g.name : g.id, stats });
  }
  return out;
}

function ModulesPage(props: { s: SessionHandle; extensions: ExtensionInfo[] }) {
  // P1-03: every loaded extension's status card (`info-screen.groups`): compactor, mcp, memory, updater, web-api…
  const [groups] = createResource(
    () => props.extensions.filter((e) => e.loaded && e.enabled).map((e) => e.id).join(','),
    async (ids) => {
      const replies = await Promise.all(
        ids
          .split(',')
          .filter(Boolean)
          .map((id) =>
            serviceCall<unknown>(props.s, 'info-screen.groups', {}, id)
              .then((reply) => parseInfoGroups(id, reply))
              .catch(() => [] as InfoGroup[]),
          ),
      );
      return replies.flat().filter((g) => g.stats.length > 0);
    },
  );
  const rows = () => props.extensions.map((e) => [e.id, e.loaded ? 'loaded' : e.enabled ? 'enabled' : 'off', e.path ?? ''] as const);
  return (
    <>
    <Show when={(groups.latest ?? []).length > 0}>
      <div class="sm-groups" data-testid="sm-info-groups">
        <For each={groups.latest ?? []}>
          {(g) => (
            <section class="sm-group" data-testid="sm-info-group" title={g.owner}>
              <div class="sm-group-name">{g.name}</div>
              <div class="sm-kv">
                <For each={g.stats}>
                  {(st) => (
                    <div class="sm-kv-row">
                      <span class="sm-kv-key">{st.label}</span>
                      <span class="sm-kv-val" data-num={/^[\d.,:\-TZ%$ ]+$/.test(st.value) ? '' : undefined} title={st.detail ? `${st.value} · ${st.detail}` : st.value}>
                        {st.value}
                        {st.detail ? ` · ${st.detail}` : ''}
                      </span>
                    </div>
                  )}
                </For>
              </div>
            </section>
          )}
        </For>
      </div>
      <div class="sm-section-title sm-section-gap">Extensions</div>
    </Show>
    <Show when={props.extensions.length > 0} fallback={<p class="sm-note">No extensions loaded.</p>}>
      <div class="sm-rows">
        <For each={rows()}>
          {([id, state, path]) => (
            <div class="sm-row">
              <span class="sm-dot" data-state={state === 'loaded' ? 'running' : state === 'enabled' ? 'idle' : 'closed'} />
              <span class="sm-row-main">
                <span class="sm-row-title" style={{ 'font-family': 'inherit' }}>{id}</span>
                <span class="sm-row-sub">{path || state}</span>
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
    </>
  );
}

export function InfoTab(props: { s: SessionHandle; initialPage?: InfoPage }) {
  const [page, setPage] = createSignal<InfoPage>(props.initialPage === 'usage' || !props.initialPage ? 'session' : props.initialPage);
  const [data, { refetch }] = createResource(liveSource(() => props.s), async (state) => {
    if (state !== 'live') return undefined;
    const s = props.s;
    const [stats, commands, extensions] = await Promise.all([
      s.cmd<unknown>({ type: 'get_session_stats' }).catch(() => undefined),
      s.cmd<{ commands?: CommandInfo[] }>({ type: 'get_commands' }).catch(() => undefined),
      s.cmd<unknown>({ type: '_oino/extensions' }).catch(() => undefined),
    ]);
    return {
      stats: parseSessionStats(stats),
      commands: Array.isArray(commands?.commands) ? commands.commands : [],
      extensions: parseExtensions(extensions),
    };
  });

  return (
    <div class="sm-info" data-testid="sm-info">
      <div class="sm-pages" role="tablist" aria-label="Info pages">
        <For each={PAGES}>
          {(p) => (
            <button type="button" role="tab" aria-selected={page() === p.id} class="sm-page" data-testid={`sm-page-${p.id}`} onClick={() => setPage(p.id)}>
              {p.label}
            </button>
          )}
        </For>
      </div>
      <Show when={page() === 'session'}>
        <SessionPage s={props.s} stats={data.latest?.stats} />
      </Show>
      <Show when={page() === 'tools'}>
        <ToolsPage s={props.s} commands={data.latest?.commands ?? []} />
      </Show>
      <Show when={page() === 'skills'}>
        <SkillsPage commands={data.latest?.commands ?? []} />
      </Show>
      <Show when={page() === 'modules'}>
        <ModulesPage s={props.s} extensions={data.latest?.extensions ?? []} />
      </Show>
      <LimitsSection s={props.s} />
    </div>
  );
}
