/**
 * [ux-sessions] The sidebar session search: the "Task list" header's search icon, `/` and
 * Ctrl+F focus the box, the query goes to the HOST (`sessions_query`, debounced, paged) with the
 * list's filters, archived chats are found and marked, Esc clears.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { DEFAULT_VIEW, resetListViews, setListView } from '../../lib/session/listView';
import { resetSidebarSearch, searchArgs } from '../../lib/session/sidebarSearch';
import { createToastStack, setToasts } from '../../lib/toasts';
import { resetHostCollapse } from './HostGroup';
import type { SessionCard } from '../../lib/host/client';
import { SessionList } from './SessionList';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, over: Partial<SessionCard> & { first_message?: string } = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
  ...over,
});

let world: ReturnType<typeof createMockHost>;
const boot = (cards: SessionCard[], archived: SessionCard[] = []) => {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards, archived }] };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/';
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => <SessionList variant="sidebar" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};
const queries = () => world.log.filter((e) => e.op === 'sessions_query').map((e) => e.args as Record<string, unknown>);

beforeEach(() => {
  localStorage.clear();
  resetHostCollapse();
  resetListViews();
  resetSidebarSearch();
  setToasts(createToastStack());
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
  setToasts(undefined);
  resetSidebarSearch();
});

describe('sidebar search', () => {
  it('asks the host (title + first message, archived too), shows the hits, Esc clears', async () => {
    const many = Array.from({ length: 300 }, (_, i) => card(`s${i}`, { name: `Chat ${i}` }));
    boot([...many, card('fm', { name: 'Unrelated', first_message: 'please fix the flaky parser test' })], [card('arch', { name: 'Old parser notes' })]);
    await waitFor(() => expect(screen.getByTestId('sb-tasks')).toBeTruthy());
    expect(screen.getByTestId('sb-tasks').textContent).toContain('Task list');
    fireEvent.click(screen.getByTestId('sb-search-toggle'));
    const input = await screen.findByTestId('sb-search-input');
    fireEvent.input(input, { target: { value: 'pars' } });
    fireEvent.input(input, { target: { value: 'parser' } });
    await waitFor(() => expect(screen.getByTestId('srow-arch')).toBeTruthy());
    // Debounced: one query for the settled text, never one per keystroke.
    expect(queries()).toHaveLength(1);
    expect(queries()[0]).toMatchObject({ query: 'parser', scope: 'all', sort: 'updated' });
    expect(screen.getByTestId('srow-fm')).toBeTruthy();
    expect(screen.getByTestId('srow-arch').hasAttribute('data-archived')).toBe(true);
    // The groups are hidden while searching; the webview never lists the 300 chats.
    expect(screen.queryByTestId('srow-s1')).toBeNull();
    expect(screen.getByTestId('sb-results-count-this-pc').textContent).toContain('2 results');
    fireEvent.keyDown(screen.getByTestId('sb-search-input'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('sb-search-input')).toBeNull());
    await waitFor(() => expect(screen.getByTestId('srow-s0')).toBeTruthy());
  });

  it('pages with Show more and respects the Properties filters', async () => {
    const many = Array.from({ length: 120 }, (_, i) => card(`m${i}`, { name: `match ${i}`, updated_at: Date.now() - i * 1000 }));
    boot([...many, card('cron1', { name: 'match cron', source: 'cron' })]);
    await waitFor(() => expect(screen.getByTestId('sb-tasks')).toBeTruthy());
    fireEvent.click(screen.getByTestId('sb-search-toggle'));
    fireEvent.input(await screen.findByTestId('sb-search-input'), { target: { value: 'match' } });
    await waitFor(() => expect(screen.getByTestId('sb-results-count-this-pc').textContent).toContain('121 results'));
    expect(document.querySelectorAll('[data-testid^="sb-results-this-pc"] .srow')).toHaveLength(50);
    fireEvent.click(screen.getByTestId('sb-results-more-this-pc'));
    await waitFor(() => expect(document.querySelectorAll('[data-testid^="sb-results-this-pc"] .srow')).toHaveLength(100));
    expect(queries().at(-1)).toMatchObject({ cursor: '50' });
    setListView('this-pc', { filters: { automation: ['cron'] } });
    await waitFor(() => expect(screen.getByTestId('sb-results-count-this-pc').textContent).toContain('1 result'));
    expect(queries().at(-1)).toMatchObject({ filters: { automation: ['cron'] } });
  });

  it('/ and Ctrl+F in the sidebar open and focus the box', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('sb-tasks')).toBeTruthy());
    fireEvent.keyDown(document.body, { key: '/' });
    const input = await screen.findByTestId('sb-search-input');
    await waitFor(() => expect(document.activeElement).toBe(input));
    fireEvent.keyDown(input, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('sb-search-input')).toBeNull());
    const row = await screen.findByTestId('srow-a');
    row.focus();
    fireEvent.keyDown(row, { key: 'f', ctrlKey: true });
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId('sb-search-input')));
  });
});

describe('searchArgs', () => {
  it('maps the view onto the host query', () => {
    const now = new Date(2026, 10, 3, 12).getTime();
    expect(searchArgs(DEFAULT_VIEW, 'x', now)).toEqual({ query: 'x', scope: 'all', sort: 'updated', filters: {} });
    const v = { ...DEFAULT_VIEW, sort: 'name' as const, filters: { ...DEFAULT_VIEW.filters, archived: ['archived' as const], workspaces: [''], status: ['running' as const], updated: { preset: 'today' as const } } };
    const a = searchArgs(v, 'y', now);
    expect(a.scope).toBe('archived');
    expect(a.sort).toBe('name');
    expect(a.filters.workspaces).toEqual(['']);
    expect(a.filters.status).toEqual(['running']);
    expect(a.filters.updated?.from).toBe(new Date(2026, 10, 3).getTime());
    expect(a.filters.updated?.to).toBeUndefined();
  });
});
