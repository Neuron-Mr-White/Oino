/**
 * Info tab (WP-A17, G-13): the `/info` pages natively — Session (identity +
 * `get_session_stats`), Usage (this PC's sessions from the hub cards), Tools
 * (`get_commands`, grouped by source), Skills (commands with source `skill`,
 * grouped like the extension groups them) and Modules (`_oino/extensions`)
 * — plus the watchdog Limits section (G-16). The info-screen extension's own
 * fallback tree stays what non-native frontends see (ui-protocol §8).
 */
import { createResource, createSignal, For, Show } from 'solid-js';
import type { SessionHandle } from '../../lib/session/store';
import { hubs } from '../../lib/session/hub';
import { formatCount, formatUsd, liveSource, parseExtensions, parseSessionStats, type CommandInfo, type ExtensionInfo, type SessionStats } from './modalData';
import { LimitsSection } from './LimitsSection';
import type { InfoPage } from './SessionModal';

const PAGES: Array<{ id: InfoPage; label: string }> = [
  { id: 'session', label: 'Session' },
  { id: 'usage', label: 'Usage' },
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

function SessionPage(props: { s: SessionHandle; stats: SessionStats | undefined }) {
  const s = () => props.s;
  const card = () => hubs().ensure(s().hostId).state.cards[s().sessionId()];
  const state = () => s().state;
  const model = () => state().model;
  const stats = () => props.stats;
  return (
    <>
      <div class="sm-grid">
        <div class="sm-stat"><span class="sm-stat-label">Cost</span><span class="sm-stat-value">{formatUsd(stats()?.cost)}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">Tokens</span><span class="sm-stat-value">{formatCount(stats()?.tokens.total ?? 0)}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">User / assistant</span><span class="sm-stat-value">{stats()?.userMessages ?? 0} / {stats()?.assistantMessages ?? 0}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">Tool calls</span><span class="sm-stat-value">{stats()?.toolCalls ?? 0}</span></div>
      </div>
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
    </>
  );
}

function UsagePage(props: { s: SessionHandle }) {
  const cards = () => Object.values(hubs().ensure(props.s.hostId).state.cards);
  const today = () => {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    return start.getTime();
  };
  const sessionsToday = () => cards().filter((c) => (c.updated_at ?? 0) >= today()).length;
  const running = () => cards().filter((c) => c.run_state === 'working').length;
  const needsYou = () => cards().filter((c) => c.needs_you).length;
  const messages = () => cards().reduce((n, c) => n + (c.message_count ?? 0), 0);
  return (
    <>
      <div class="sm-grid">
        <div class="sm-stat"><span class="sm-stat-label">Sessions</span><span class="sm-stat-value">{cards().length}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">Today</span><span class="sm-stat-value">{sessionsToday()}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">Running</span><span class="sm-stat-value">{running()}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">Needs you</span><span class="sm-stat-value">{needsYou()}</span></div>
        <div class="sm-stat"><span class="sm-stat-label">Messages</span><span class="sm-stat-value">{formatCount(messages())}</span></div>
      </div>
      <p class="sm-note">Across the sessions this PC reports. Per-day token and cost history is the host's to expose.</p>
    </>
  );
}

function ToolsPage(props: { commands: CommandInfo[] }) {
  const groups = () => groupBy(props.commands.filter((c) => c.source !== 'skill'), commandSource);
  return (
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

function ModulesPage(props: { extensions: ExtensionInfo[] }) {
  const rows = () => props.extensions.map((e) => [e.id, e.loaded ? 'loaded' : e.enabled ? 'enabled' : 'off', e.path ?? ''] as const);
  return (
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
  );
}

export function InfoTab(props: { s: SessionHandle; initialPage?: InfoPage }) {
  const [page, setPage] = createSignal<InfoPage>(props.initialPage ?? 'session');
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
    <div data-testid="sm-info">
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
      <Show when={page() === 'usage'}>
        <UsagePage s={props.s} />
      </Show>
      <Show when={page() === 'tools'}>
        <ToolsPage commands={data.latest?.commands ?? []} />
      </Show>
      <Show when={page() === 'skills'}>
        <SkillsPage commands={data.latest?.commands ?? []} />
      </Show>
      <Show when={page() === 'modules'}>
        <ModulesPage extensions={data.latest?.extensions ?? []} />
      </Show>
      <LimitsSection s={props.s} />
    </div>
  );
}
