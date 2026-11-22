/**
 * Big session stores in the sidebar (perf-sessions): a skeleton while the first
 * listing loads, project folders paged ("Show more projects", the shown chat's
 * project always kept) and the Archived list paged.
 */
import { resetListViews, setListView } from '../../lib/session/listView';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import type { HostClient, SessionCard } from '../../lib/host/client';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { PROJECTS_PAGE, resetHostCollapse, visibleProjects } from './HostGroup';
import { ARCHIVED_PAGE } from './ArchivedList';
import { SessionList } from './SessionList';

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

/** `projects` folders with 2 sessions each, newest folder first; `archived` trashed chats. */
const scenario = (projects: number, archived = 0): MockScenario => ({
  name: 'long',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: Array.from({ length: projects }, (_, p) =>
        [0, 1].map((n) => card(`p${p}-${n}`, { cwd: `/home/you/p/proj${p}`, updated_at: 1_000_000 - p * 10 - n })),
      ).flat(),
      archived: Array.from({ length: archived }, (_, n) => card(`a${n}`, { updated_at: 500_000 - n })),
    },
  ],
});

/** The mock client with `sessions_list` held until `release()`. */
function gated(world: ReturnType<typeof createMockHost>) {
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  const factory = (hostId: string): HostClient =>
    new Proxy(world.client(hostId), {
      get(target, key, receiver) {
        if (key === 'request') {
          return async (op: string, ...rest: unknown[]) => {
            if (op === 'sessions_list') await gate;
            return (target.request as (...a: unknown[]) => unknown).call(target, op, ...rest);
          };
        }
        const value = Reflect.get(target, key, receiver) as unknown;
        return typeof value === 'function' ? (value as (...a: unknown[]) => unknown).bind(target) : value;
      },
    }) as HostClient;
  return { factory, release };
}

function mount(hash = '#/') {
  location.hash = hash;
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => <SessionList variant="sidebar" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
}

const projectsShown = (container: HTMLElement) => container.querySelectorAll('.sb-project:not(.sb-pinned)').length;

beforeEach(() => {
  localStorage.clear();
  resetHostCollapse();
  resetListViews();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
  location.hash = '';
});

describe('sidebar with a big store', () => {
  it('shows skeleton rows while the first listing loads, then the sessions', async () => {
    const world = createMockHost(scenario(3));
    const { factory, release } = gated(world);
    setHostClientFactory(factory);
    mount();
    const skeleton = await screen.findByTestId('sb-skeleton-this-pc');
    expect(skeleton.getAttribute('role')).toBe('status');
    expect(skeleton.querySelectorAll('.sb-skel-row').length).toBeGreaterThan(2);
    expect(screen.queryByText('No sessions yet')).toBeNull();
    release();
    await waitFor(() => expect(screen.queryByTestId('sb-skeleton-this-pc')).toBeNull());
    expect(screen.getByTestId('sb-rows-proj0')).toBeTruthy();
  });

  it('pages project folders: Show more projects adds a page, regrouping resets', async () => {
    setHostClientFactory(createMockHost(scenario(PROJECTS_PAGE * 2 + 3)).client);
    const { container } = mount();
    await waitFor(() => expect(screen.getByTestId('sb-rows-proj0')).toBeTruthy());
    expect(projectsShown(container)).toBe(PROJECTS_PAGE);
    const more = screen.getByTestId('sb-more-projects-this-pc');
    expect(more.textContent).toContain(String(PROJECTS_PAGE + 3));
    fireEvent.click(more);
    await waitFor(() => expect(projectsShown(container)).toBe(PROJECTS_PAGE * 2));
    fireEvent.click(screen.getByTestId('sb-more-projects-this-pc'));
    await waitFor(() => expect(projectsShown(container)).toBe(PROJECTS_PAGE * 2 + 3));
    expect(screen.queryByTestId('sb-more-projects-this-pc')).toBeNull();
    // [ux-sessions] one host = no host header to collapse; regrouping resets the paging.
    expect(screen.queryByTestId('sb-host-head-this-pc')).toBeNull();
    setListView('this-pc', { group: 'none' });
    setListView('this-pc', { group: 'project' });
    await waitFor(() => expect(projectsShown(container)).toBe(PROJECTS_PAGE));
  });

  it("keeps the shown chat's project visible beyond the first page", async () => {
    const last = PROJECTS_PAGE + 4;
    setHostClientFactory(createMockHost(scenario(last + 1)).client);
    const { container } = mount(`#/s/this-pc/p${last}-0`);
    await waitFor(() => expect(screen.getByTestId(`sb-rows-proj${last}`)).toBeTruthy());
    expect(projectsShown(container)).toBe(PROJECTS_PAGE + 1);
  });

});

describe('visibleProjects', () => {
  const groups = Array.from({ length: 5 }, (_, g) => ({ cwd: `/p${g}`, cards: [{ session_id: `s${g}` }] }));
  it('takes the first page, plus the active session’s group', () => {
    expect(visibleProjects(groups, 10).length).toBe(5);
    expect(visibleProjects(groups, 2).map((g) => g.cwd)).toEqual(['/p0', '/p1']);
    expect(visibleProjects(groups, 2, 's4').map((g) => g.cwd)).toEqual(['/p0', '/p1', '/p4']);
    expect(visibleProjects(groups, 2, 's1').map((g) => g.cwd)).toEqual(['/p0', '/p1']);
  });
});
