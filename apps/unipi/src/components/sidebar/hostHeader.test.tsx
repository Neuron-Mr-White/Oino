/**
 * [ux-sessions] (owner 2026-11-03) One host = no "THIS COMPUTER (n)" header: the Properties icon
 * lives in the Task list header; a merged view of several hosts keeps a small label per host. The
 * empty state never shows while the list is loading or mid-batch.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { hubs, resetHubs } from '../../lib/session/hub';
import { resetListViews } from '../../lib/session/listView';
import { resetSidebarSearch } from '../../lib/session/sidebarSearch';
import { createToastStack, setToasts } from '../../lib/toasts';
import type { SessionCard } from '../../lib/host/client';
import { resetHostCollapse } from './HostGroup';
import { SessionList } from './SessionList';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({ session_id: id, name: `Session ${id}`, cwd: '/home/you/p/oino', run_state: 'idle', updated_at: Date.now() - 90_000, ...over });

const mount = (scenario: MockScenario) => {
  const world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/';
  render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => <SessionList variant="sidebar" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
  return world;
};

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
});

describe('host header', () => {
  it('one host: no host header; the Task list filter icon opens its Properties', async () => {
    mount({ name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: [card('a'), card('b')] }] });
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    expect(screen.queryByTestId('sb-host-head-this-pc')).toBeNull();
    expect(screen.queryByTestId('sb-props-this-pc')).toBeNull();
    expect(document.body.textContent).not.toMatch(/this computer/i);
    fireEvent.click(screen.getByTestId('sb-tasks-props'));
    await waitFor(() => expect(screen.getByTestId('lp-archive-all')).toBeTruthy());
  });

  it('several hosts listed at once keep a small label each', async () => {
    mount({
      name: 't',
      hosts: [
        { id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: [card('a')] },
        { id: 'pc-x', name: 'coffee', conn: { state: 'connected' }, cards: [card('x1')] },
      ],
    });
    await waitFor(() => expect(screen.getByTestId('sb-host-head-this-pc')).toBeTruthy());
    expect(screen.getByTestId('sb-host-head-pc-x').textContent).toContain('coffee');
  });

  it('never says "No sessions yet" while the list loads or a batch runs', async () => {
    const world = mount({ name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: [card('a')] }] });
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    const hub = hubs().ensure('this-pc');
    let release!: () => void;
    // Hold the batch reply open: the rows are gone, the progress line shows, no empty state.
    const gate = new Promise<void>((r) => (release = r));
    const client = world.client('this-pc');
    const original = client.request.bind(client);
    client.request = (async (op: string, args?: Record<string, unknown>) => {
      if (op === 'sessions_archive') await gate;
      return original(op, args);
    }) as typeof client.request;
    const run = hub.archiveMany(['a']);
    await waitFor(() => expect(screen.getByTestId('sb-bulk-this-pc')).toBeTruthy());
    expect(screen.queryByTestId('sb-empty-this-pc')).toBeNull();
    release();
    await run;
    await waitFor(() => expect(screen.getByTestId('sb-empty-this-pc')).toBeTruthy());
  });
});
