/**
 * [ui-sessions] The sessions list's Properties menu (Devin-style sort / filter /
 * group / property visibility / compact / subagents), "Archive all" (list +
 * group header, confirm with the count, Undo toast), the one-meta-line row
 * and the opt-in subagents tree — against the mock host, like sessionList.test.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { hubs, resetHubs } from '../../lib/session/hub';
import { listView, LIST_VIEW_KEY, resetListViews, toggleSubagentRow } from '../../lib/session/listView';
import { createToastStack, setToasts, toasts } from '../../lib/toasts';
import { resetHostCollapse } from './HostGroup';
import type { SessionCard } from '../../lib/host/client';
import { SessionList } from './SessionList';
import { SessionMenu } from './SessionMenu';
import { formatDuration, runsFromAgents, runsFromTopic } from './SubagentTree';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
  ...over,
});

let world: ReturnType<typeof createMockHost>;
const boot = (cards: SessionCard[]) => {
  const scenario: MockScenario = {
    name: 't',
    hosts: [
      { id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards },
      { id: 'pc-x', name: 'coffee', conn: { state: 'connected' }, cards: [card('x1', { cwd: '/home/me/other' })] },
    ],
  };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/';
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route
            path="*"
            component={() => (
              <>
                <SessionList variant="sidebar" />
                <SessionMenu />
              </>
            )}
          />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const rowIds = (base: ParentNode) => [...base.querySelectorAll('.srow')].map((r) => r.getAttribute('data-testid')!.slice('srow-'.length));
const openProps = () => fireEvent.click(screen.getByTestId('sb-props-this-pc'));
const host = () => screen.getByTestId('sb-host-this-pc');

beforeEach(() => {
  localStorage.clear();
  resetHostCollapse();
  resetListViews();
  setToasts(createToastStack());
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
  setToasts(undefined);
});

describe('Properties menu', () => {
  it('groups by status, sorts by name, and persists per host', async () => {
    boot([card('b-run', { run_state: 'working', name: 'Beta' }), card('a-idle', { name: 'Alpha' }), card('c-idle', { name: 'Charlie', cwd: '/home/you/p/web' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    openProps();
    fireEvent.click(screen.getByTestId('lp-group'));
    fireEvent.click(screen.getByTestId('lp-group-status'));
    await waitFor(() => expect(screen.getByTestId('sb-rows-Running')).toBeTruthy());
    expect(rowIds(screen.getByTestId('sb-rows-Running'))).toEqual(['b-run']);
    fireEvent.click(screen.getByTestId('lp-back'));
    fireEvent.click(screen.getByTestId('lp-sort'));
    fireEvent.click(screen.getByTestId('lp-sort-name'));
    expect(rowIds(screen.getByTestId('sb-rows-Ready'))).toEqual(['a-idle', 'c-idle']);
    // Only this host changed; the view survives a reload of the store.
    expect(listView('pc-x').group).toBe('project');
    const stored = JSON.parse(localStorage.getItem(LIST_VIEW_KEY)!) as Record<string, { group: string; sort: string }>;
    expect(stored['this-pc']).toMatchObject({ group: 'status', sort: 'name' });
  });

  it('filters show a count on the button; an empty result says "No sessions found" with Reset filters', async () => {
    boot([card('m1'), card('c1', { source: 'cron', name: 'Nightly' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    openProps();
    fireEvent.click(screen.getByTestId('lp-filter'));
    fireEvent.click(screen.getByTestId('lp-facet-automation'));
    fireEvent.click(screen.getByTestId('lp-automation-cron'));
    await waitFor(() => expect(rowIds(host())).toEqual(['c1']));
    const btn = screen.getByTestId('sb-props-this-pc');
    expect(btn.hasAttribute('data-active')).toBe(true);
    expect(btn.textContent).toBe('1');
    expect(within(host()).getByText('1/2')).toBeTruthy();
    // Status filter that nothing matches → empty state.
    fireEvent.click(screen.getByTestId('lp-back'));
    fireEvent.click(screen.getByTestId('lp-facet-status'));
    fireEvent.click(screen.getByTestId('lp-status-running'));
    await waitFor(() => expect(screen.getByTestId('sb-no-match-this-pc').textContent).toContain('No sessions found'));
    fireEvent.click(screen.getByTestId('sb-reset-filters-this-pc'));
    await waitFor(() => expect(rowIds(host()).sort()).toEqual(['c1', 'm1']));
    expect(btn.hasAttribute('data-active')).toBe(false);
  });

  it('"Search filters" narrows the facet list; Workspace is a searchable multi-select', async () => {
    boot([card('o1'), card('w1', { cwd: '/home/you/p/website' }), card('d1', { cwd: '/home/you/p/docs' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    openProps();
    fireEvent.click(screen.getByTestId('lp-filter'));
    fireEvent.input(screen.getByTestId('lp-search'), { target: { value: 'work' } });
    expect(screen.queryByTestId('lp-facet-status')).toBeNull();
    fireEvent.click(screen.getByTestId('lp-facet-workspaces'));
    fireEvent.input(screen.getByTestId('lp-search'), { target: { value: 'web' } });
    expect(screen.queryByTestId('lp-ws-docs')).toBeNull();
    fireEvent.click(screen.getByTestId('lp-ws-website'));
    fireEvent.input(screen.getByTestId('lp-search'), { target: { value: '' } });
    fireEvent.click(screen.getByTestId('lp-ws-docs'));
    await waitFor(() => expect(rowIds(host()).sort()).toEqual(['d1', 'w1']));
  });

  it('compact view puts each row on one line; property visibility hides the time; Hide all hides every chip', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    // [sidebar-ui] Normal rows: the title line + a second meta line (the age lives there).
    expect(screen.getByTestId('srow-a').querySelector('.srow-meta .srow-time')).toBeTruthy();
    openProps();
    fireEvent.click(screen.getByTestId('lp-compact'));
    await waitFor(() => expect(screen.getByTestId('srow-a').hasAttribute('data-compact')).toBe(true));
    // Compact: the title and nothing else — no meta line, no age (owner-fix-8).
    expect(screen.getByTestId('srow-a').querySelector('.srow-meta')).toBeNull();
    expect(screen.getByTestId('srow-a').querySelector('.srow-line > .srow-time')).toBeNull();
    fireEvent.click(screen.getByTestId('lp-props'));
    fireEvent.click(screen.getByTestId('lp-prop-updated'));
    await waitFor(() => expect(screen.getByTestId('srow-a').querySelector('.srow-time')).toBeNull());
    fireEvent.click(screen.getByTestId('lp-props-hide-all'));
    expect(Object.values(listView('this-pc').props).every((v) => !v)).toBe(true);
  });
});

/** The ids of the one `sessions_archive` batch (asserts there was exactly one). */
const batchArchived = () => {
  const calls = world.log.filter((e) => e.op === 'sessions_archive');
  return calls.length === 1 ? [...(calls[0]!.args as { sessions: string[] }).sessions].sort() : calls.length;
};

describe('Archive all', () => {
  it('[ux-sessions] 2,000 sessions: one batched host op, the list empties at once, the open Archived list counts all of them without a reload', async () => {
    const many = Array.from({ length: 2000 }, (_, i) => card(`s${i}`, { cwd: `/home/you/p/proj${i % 40}`, updated_at: Date.now() - i * 60_000 }));
    boot(many);
    await waitFor(() => expect(screen.getByTestId('srow-s0')).toBeTruthy());
    // The owner's state: an Archived list already loaded (here: empty) before Archive all.
    void hubs().ensure('this-pc').loadArchived();
    await waitFor(() => expect(world.log.some((e) => e.op === 'sessions_list' && (e.args as { archived?: boolean } | undefined)?.archived)).toBe(true));
    openProps();
    fireEvent.click(screen.getByTestId('lp-archive-all'));
    expect(screen.getByTestId('archive-all-confirm').textContent).toContain('2000');
    fireEvent.click(screen.getByTestId('archive-all-yes'));
    await waitFor(() => expect(batchArchived()).toHaveLength(2000));
    expect(world.log.filter((e) => e.op === 'session_archive')).toHaveLength(0);
    await waitFor(() => expect(hubs().ensure('this-pc').state.archived.length).toBe(2000));
    await waitFor(() => expect(screen.queryByTestId('srow-s0')).toBeNull());
    await waitFor(() => expect(toasts().list().find((t) => t.message.startsWith('Archived 2000'))).toBeTruthy());
  });

  it('project head menu: Project settings, Archive and Dream (right-click opens it too)', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    fireEvent.contextMenu(screen.getAllByTestId('sb-project-head')[0]!);
    expect(await screen.findByTestId('sb-project-settings')).toBeTruthy();
    expect(screen.getByTestId('sb-project-archive-oino')).toBeTruthy();
    expect(screen.getByTestId('sb-project-dream')).toBeTruthy();
  });

  it('group header: confirm with the count (pinned and running kept), then an Undo toast restores them', async () => {
    boot([card('a'), card('b'), card('p', { pinned: true }), card('r', { run_state: 'working' }), card('w', { cwd: '/home/you/p/web' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    fireEvent.click(screen.getByTestId('sb-project-menu-oino'));
    fireEvent.click(await screen.findByTestId('sb-project-archive-oino'));
    const confirm = await screen.findByTestId('archive-all-confirm');
    expect(confirm.textContent).toContain('Archive 2 sessions in oino');
    fireEvent.click(screen.getByTestId('archive-all-yes'));
    await waitFor(() => expect(batchArchived()).toEqual(['a', 'b']));
    expect(world.log.filter((e) => e.op === 'session_archive')).toHaveLength(0);
    await waitFor(() => expect(screen.queryByTestId('srow-a')).toBeNull());
    expect(screen.getByTestId('srow-w')).toBeTruthy();
    await waitFor(() => expect(toasts().list().find((t) => t.message.startsWith('Archived 2'))?.action?.label).toBe('Undo'));
    const toast = toasts().list().find((t) => t.message.startsWith('Archived 2'));
    await toast!.action!.run();
    await waitFor(() => expect(world.log.filter((e) => e.op === 'sessions_restore').map((e) => (e.args as { sessions: string[] }).sessions.sort())).toEqual([['a', 'b']]));
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
  });

  it('list header: "Archive all shown" respects the active filter', async () => {
    boot([card('m1'), card('c1', { source: 'cron' }), card('c2', { source: 'cron', cwd: '/home/you/p/web' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    openProps();
    fireEvent.click(screen.getByTestId('lp-filter'));
    fireEvent.click(screen.getByTestId('lp-facet-automation'));
    fireEvent.click(screen.getByTestId('lp-automation-cron'));
    fireEvent.click(screen.getByTestId('lp-back'));
    fireEvent.click(screen.getByTestId('lp-back'));
    const all = screen.getByTestId('lp-archive-all');
    expect(all.textContent).toContain('Archive all shown');
    expect(all.textContent).toContain('2');
    fireEvent.click(all);
    expect(screen.getByTestId('archive-all-confirm').textContent).toContain('Archive 2 sessions shown');
    fireEvent.click(screen.getByTestId('archive-all-yes'));
    await waitFor(() => expect(batchArchived()).toEqual(['c1', 'c2']));
  });
});

describe('rows', () => {
  it('[ux-sessions] a cron chat is one line whose row icon is the clock (the kind), then the age', async () => {
    boot([card('c', { source: 'cron', name: 'Nightly' }), card('n', { name: 'Normal' })]);
    const row = await screen.findByTestId('srow-c');
    expect(row.querySelector('.srow-sub')).toBeNull();
    const glyph = row.querySelector('.srow-glyph')!;
    expect(glyph.getAttribute('data-kind')).toBe('cron');
    expect(glyph.querySelector('svg')).toBeTruthy();
    expect(row.querySelector('.srow-chips .srow-cron')).toBeNull();
    expect(row.querySelector('.srow-time')!.textContent).toBe('1m');
    // A normal chat has no row icon at all (no chat bubble).
    const normal = screen.getByTestId('srow-n');
    expect(normal.querySelector('.srow-glyph svg')).toBeNull();
    expect(normal.querySelector('.srow-glyph')!.hasAttribute('data-kind')).toBe(false);
  });

  it('a running row keeps the fixed glyph slot (spinner, no extra line)', async () => {
    boot([card('r', { run_state: 'working' })]);
    const row = await screen.findByTestId('srow-r');
    expect(row.querySelector('.srow-glyph .srow-spin')).toBeTruthy();
    expect(row.querySelector('.srow-sub')).toBeNull();
    // Running keeps its spinner + "Working" on the title line (the age stays on the meta line).
    expect(row.querySelector('.srow-line .srow-word')!.textContent).toBe('Working');
    expect(row.querySelector('.srow-line .srow-time')).toBeNull();
  });
});

describe('subagents tree', () => {
  it('is off by default; the row menu toggle shows the tree under that chat', async () => {
    boot([card('a'), card('b')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    expect(screen.queryByTestId('sa-tree-a')).toBeNull();
    fireEvent.contextMenu(screen.getByTestId('srow-a'));
    fireEvent.click(await screen.findByTestId('srow-subagents'));
    await waitFor(() => expect(screen.getByTestId('sa-tree-a')).toBeTruthy());
    expect(screen.queryByTestId('sa-tree-b')).toBeNull();
    // Nothing loaded yet: an explicit load, never an automatic stream open.
    expect(screen.getByTestId('sa-load-a')).toBeTruthy();
    toggleSubagentRow('this-pc', 'a');
    await waitFor(() => expect(screen.queryByTestId('sa-tree-a')).toBeNull());
  });

  it('reads the subagents topic and the _oino/agents fallback', () => {
    const runs = runsFromTopic({
      records: [
        { id: 'r1', title: 'scout', profile: 'minimax-scout', status: 'completed', startedAt: 1000, endedAt: 65_000 },
        { id: 'r2', task: 'build the thing', status: 'running', startedAt: 2000 },
        { title: 'no id' },
      ],
    })!;
    expect(runs.map((r) => [r.id, r.title, r.status])).toEqual([
      ['r2', 'build the thing', 'running'],
      ['r1', 'scout', 'completed'],
    ]);
    expect(runsFromTopic({})).toBeUndefined();
    expect(runsFromAgents([{ id: 'root', depth: 0 }, { id: 'k', label: 'kid', role: 'explore', state: 'running', depth: 1 }])).toEqual([
      { id: 'k', title: 'kid', profile: 'explore', status: 'running' },
    ]);
    expect(formatDuration(64_000)).toBe('1m 04s');
    expect(formatDuration(3_720_000)).toBe('1h 02m');
  });
});
